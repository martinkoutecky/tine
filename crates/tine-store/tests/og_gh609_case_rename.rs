//! GH #609: one rename changes spelling without a second page identity.
use std::fs;
use tine_graph_features::pages::{rename_or_merge_page, RenameOutcome};
use tine_store::Store;

#[test]
fn case_only_rename_changes_filename_title_refs_and_namespace() {
    for (ext, header, block) in [
        ("md", "title:: my note\n", "- "),
        ("org", "#+TITLE: my note\n", "* "),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::create_dir(dir.path().join("logseq")).unwrap();
        fs::write(
            dir.path().join("logseq/config.edn"),
            "{:file/name-format :triple-lowbar :default-home {:page \"my note\"}}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join(format!("pages/my note.{ext}")),
            format!("{header}{block}[[my note]]\n"),
        )
        .unwrap();
        fs::write(dir.path().join("pages/my note___child.md"), "- child\n").unwrap();
        fs::write(
            dir.path().join("pages/Refs.md"),
            "- [[my note]] [[my note/child]]\n",
        )
        .unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let report = rename_or_merge_page(&store, "my note", "My Note", None, None, &[]).unwrap();
        assert_eq!(report.outcome, RenameOutcome::Renamed);
        assert_eq!(report.home_page.as_deref(), Some("My Note"));
        let names: Vec<_> = fs::read_dir(dir.path().join("pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(names.contains(&format!("My Note.{ext}").into()));
        assert!(!names.contains(&format!("my note.{ext}").into()));
        assert!(names.contains(&"My Note___child.md".into()));
        let text = fs::read_to_string(dir.path().join(format!("pages/My Note.{ext}"))).unwrap();
        assert!(text.contains("My Note"));
        assert!(!text.contains("my note"));
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Refs.md")).unwrap(),
            "- [[My Note]] [[My Note/child]]\n"
        );
        let page = store
            .page_named("My Note", tine_core::model::PageKind::Page)
            .unwrap()
            .unwrap();
        assert_eq!(page.doc.name, "My Note");
        store.close();
        let reopened = Store::open(dir.path(), Default::default()).unwrap().0;
        assert_eq!(
            reopened
                .page_named("my note", tine_core::model::PageKind::Page)
                .unwrap()
                .unwrap()
                .doc
                .name,
            "My Note"
        );
        reopened.close();
    }
}

#[test]
fn case_move_io_failure_restores_source_and_preserves_refs() {
    use tine_store::FaultPoint;
    for point in [
        FaultPoint::CaseMoveStageIo,
        FaultPoint::DirectorySyncIo,
        FaultPoint::MoveAfterTrashCopyIo,
        FaultPoint::MidStepIoAt(0),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "- [[old]]\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        store.inject_fault(point);
        assert!(rename_or_merge_page(&store, "old", "Old", None, None, &[]).is_err());
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
fn distinct_case_twins_refuse_without_writing() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- first\n").unwrap();
    let Ok(mut twin) = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.path().join("pages/Old.md"))
    else {
        return;
    };
    twin.write_all(b"- second\n").unwrap();
    drop(twin);
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    assert!(rename_or_merge_page(&store, "old", "Old", None, None, &[]).is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
        "- first\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
        "- second\n"
    );
    store.close();
}

#[test]
fn case_rename_crash_worker() {
    let Ok(root) = std::env::var("TINE_QD1_CRASH_ROOT") else {
        return;
    };
    let store = Store::open(std::path::Path::new(&root), Default::default())
        .unwrap()
        .0;
    let point = match std::env::var("TINE_QD1_CRASH_POINT").unwrap().as_str() {
        "stage" => tine_store::FaultPoint::AbortAfterCaseMoveStage,
        "publish" => tine_store::FaultPoint::AbortAfterCaseMovePublish,
        "rewrite" => tine_store::FaultPoint::AbortAfterMoveRewrite,
        _ => unreachable!(),
    };
    store.inject_fault(point);
    rename_or_merge_page(&store, "old", "Old", None, None, &[]).unwrap();
    panic!("case rename did not reach abort boundary");
}

#[test]
fn case_rename_kill_reopen_preserves_bytes_and_can_resume() {
    use tine_store::{Area, EditKind, TxOutcome};
    for point in ["stage", "publish", "rewrite"] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "title:: old\n- [[old]]\n").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "case_rename_crash_worker"])
            .env("TINE_QD1_CRASH_ROOT", dir.path())
            .env("TINE_QD1_CRASH_POINT", point)
            .output()
            .unwrap();
        assert!(!output.status.success());
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                output.status.signal(),
                Some(6),
                "must abort, not panic: {output:?}"
            );
        }
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        if point == "stage" {
            assert_eq!(fs::read_dir(dir.path().join("pages")).unwrap().count(), 0);
            let trash = fs::read_dir(dir.path().join("logseq/.tine-trash/pages"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            assert_eq!(
                fs::read_to_string(&trash).unwrap(),
                "title:: old\n- [[old]]\n"
            );
            // Recover through the same guarded store move door, no raw write.
            let rel = format!("pages/{}", trash.file_name().unwrap().to_str().unwrap());
            let source = store.file_id(Area::Trash, &rel).unwrap();
            let rev = store.read(&source, None).unwrap().1;
            let to = store.file_id(Area::Pages, "old.md").unwrap();
            let mut tx = store.transaction(Some(EditKind::RenamePage));
            tx.move_file(&source, rev, &to, None);
            assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
        } else {
            let expected = if point == "publish" {
                "title:: old\n- [[old]]\n"
            } else {
                "title:: Old\n- [[Old]]\n"
            };
            assert_eq!(
                fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
                expected
            );
        }
        rename_or_merge_page(&store, "old", "Old", None, None, &[]).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
            "title:: Old\n- [[Old]]\n"
        );
        assert_eq!(fs::read_dir(dir.path().join("pages")).unwrap().count(), 1);
        store.close();
    }
}

#[test]
fn a_competing_destination_between_case_moves_is_never_overwritten() {
    use tine_store::{Area, EditKind, FaultPoint, TxOutcome};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- original\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let src = store.file_id(Area::Pages, "old.md").unwrap();
    let dst = store.file_id(Area::Pages, "Old.md").unwrap();
    let rev = store.read(&src, None).unwrap().1;
    store.inject_fault(FaultPoint::CaseMoveStageCollision);
    let mut tx = store.transaction(Some(EditKind::RenamePage));
    tx.move_file(&src, rev, &dst, None);
    assert!(matches!(tx.commit(), TxOutcome::NotCommitted { .. }));
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
        "external collision"
    );
    // On folding disks the competing spelling also occupies the original;
    // rollback must preserve original bytes in trash instead of replacing it.
    let listed = fs::read_dir(dir.path().join("pages"))
        .unwrap()
        .any(|entry| entry.unwrap().file_name() == "old.md");
    if listed {
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
            "- original\n"
        );
    } else {
        assert!(fs::read_dir(dir.path().join("logseq/.tine-trash/pages"))
            .unwrap()
            .any(|entry| fs::read(entry.unwrap().path()).unwrap() == b"- original\n"));
    }
    store.close();
}

#[cfg(unix)]
#[test]
fn separately_listed_hardlinked_case_destination_is_not_an_alias_spelling() {
    use tine_store::{Area, EditKind, TxOutcome};
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
    let mut tx = store.transaction(Some(EditKind::RenamePage));
    tx.move_file(&src, rev, &dst, None);
    assert!(matches!(tx.commit(), TxOutcome::NotCommitted { .. }));
    assert_eq!(fs::read_dir(dir.path().join("pages")).unwrap().count(), 2);
    store.close();
}

#[test]
fn case_rename_uses_the_shared_unicode_identity_on_normalizing_filesystems() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    // macOS can return a decomposed physical filename for a composed title.
    fs::write(
        dir.path().join("pages/cafe\u{301}.md"),
        "title:: café\n- [[café]]\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    assert_eq!(
        rename_or_merge_page(&store, "café", "CAFÉ", None, None, &[])
            .unwrap()
            .outcome,
        RenameOutcome::Renamed
    );
    assert_eq!(
        store
            .page_named("CAFÉ", tine_core::model::PageKind::Page)
            .unwrap()
            .unwrap()
            .doc
            .name,
        "CAFÉ"
    );
    assert_eq!(fs::read_dir(dir.path().join("pages")).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/CAFÉ.md")).unwrap(),
        "title:: CAFÉ\n- [[CAFÉ]]\n"
    );
    store.close();
}

#[test]
fn successful_case_rename_retains_only_the_existing_old_byte_copy() {
    for blocks in [1, 60] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let old = "- [[old]]\n".repeat(blocks);
        fs::write(dir.path().join("pages/old.md"), &old).unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        rename_or_merge_page(&store, "old", "Old", None, None, &[]).unwrap();
        let copies: Vec<_> = fs::read_dir(dir.path().join("logseq/.tine-trash/pages"))
            .unwrap()
            .map(|entry| fs::read(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(copies, [old.as_bytes()]);
        let live = fs::read(dir.path().join("pages/Old.md")).unwrap();
        assert_eq!(live, "- [[Old]]\n".repeat(blocks).as_bytes());
        eprintln!("case rename blocks={blocks}: live={} B, old-copy={} B, retained copies=1, staging copies=0", live.len(), old.len());
        store.close();
    }
}
