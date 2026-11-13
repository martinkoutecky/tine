use crate::{Area, FaultPoint, OpenOptions, PageId, RenameMap, SaveBase, Store};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tine_core::model::{BlockDto, Format, PageDto, PageKind};

fn scratch(label: &str) -> PathBuf {
    let root = crate::test_support::fresh_dir(&format!("tine-i02-{label}-"));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn open(root: &Path) -> std::sync::Arc<Store> {
    std::sync::Arc::new(Store::open(root, OpenOptions::default()).unwrap().0)
}

fn page(name: &str, raw: &str) -> PageDto {
    PageDto {
        name: name.into(),
        title: name.into(),
        kind: PageKind::Page,
        format: Format::Md,
        blocks: vec![BlockDto {
            id: "crash-block".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        pre_block: None,
        rev: None,
        read_only: false,
        guide: false,
    }
}

fn recovery_has(root: &Path, expected: &[u8]) -> bool {
    fn walk(dir: &Path, expected: &[u8]) -> bool {
        let Ok(entries) = fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, expected)
            } else {
                fs::read(path).is_ok_and(|bytes| bytes == expected)
            }
        })
    }
    walk(&root.join("logseq/.tine-trash"), expected)
        || walk(&root.join("assets/.tine-restore-recovery"), expected)
}

fn child(root: &Path, worker: &str, boundary: usize) {
    let output = Command::new(tine_store::test_binary::own_test_binary())
        .arg("--exact")
        .arg(worker)
        .arg("--nocapture")
        .env("TINE_CRASH_ROOT", root)
        .env("TINE_CRASH_BOUNDARY", boundary.to_string())
        .output()
        .unwrap();
    assert!(!output.status.success(),
        "I-2: child must abort at durable boundary {boundary}; exemplar Transaction::commit / Store::restore: {}",
        String::from_utf8_lossy(&output.stdout));
}

#[test]
fn crash_transaction_worker() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let Ok(root) = std::env::var("TINE_CRASH_ROOT") else {
        return;
    };
    let Ok(boundary) = std::env::var("TINE_CRASH_BOUNDARY") else {
        return;
    };
    if std::env::var("TINE_CRASH_KIND").as_deref() == Ok("restore") {
        return;
    }
    let root = Path::new(&root);
    let store = open(root);
    let a = store.file_id(Area::Assets, "A.md").unwrap();
    let b = store.file_id(Area::Assets, "B.md").unwrap();
    let d = store.file_id(Area::Assets, "D.md").unwrap();
    let c = store.file_id(Area::Assets, "c.bin").unwrap();
    let a_rev = store.read(&a, None).unwrap().1;
    let b_rev = store.read(&b, None).unwrap().1;
    let c_rev = store.read(&c, None).unwrap().1;
    let mut tx = store.transaction(None);
    tx.replace(&a, a_rev, b"- new A\n".to_vec());
    tx.move_file(&b, b_rev, &d, None);
    tx.trash(&c, c_rev, crate::TrashIf::Any);
    store.inject_fault(FaultPoint::AbortAfterStep(boundary.parse().unwrap()));
    let _ = tx.commit();
    panic!("I-2: transaction fault did not abort; exemplar Transaction::commit");
}

#[test]
fn transaction_kill_reopen_preserves_each_old_or_new_file() {
    let _integration_guard = crate::test_transactions::integration_lock();
    for boundary in 0..3 {
        let root = scratch("tx");
        fs::write(root.join("assets/A.md"), b"- old A\n").unwrap();
        fs::write(root.join("assets/B.md"), b"- old B\n").unwrap();
        fs::write(root.join("assets/c.bin"), b"old C").unwrap();
        child(
            &root,
            "test_transactions::i02_crash_gaps::crash_transaction_worker",
            boundary,
        );
        let reopened = open(&root);
        reopened.whole_graph().unwrap();
        let a = fs::read(root.join("assets/A.md")).unwrap();
        assert!(a == b"- old A\n" || a == b"- new A\n",
            "I-2: no torn page after commit crash; exemplar Transaction::commit at {boundary}: {a:?}");
        let b = fs::read(root.join("assets/B.md")).ok();
        let d = fs::read(root.join("assets/D.md")).ok();
        assert!(
            b.as_deref() == Some(b"- old B\n") || d.as_deref() == Some(b"- old B\n"),
            "I-2: moved page survives at boundary {boundary}; exemplar Transaction::commit"
        );
        assert!(fs::read(root.join("assets/c.bin")).is_ok_and(|bytes| bytes == b"old C")
            || recovery_has(&root, b"old C"),
            "I-2: trashed asset remains live or recoverable at boundary {boundary}; exemplar Transaction::commit");
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn crash_receiver_before_source_worker() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let Ok(root) = std::env::var("TINE_CRASH_REF_ROOT") else {
        return;
    };
    let source_name = std::env::var("TINE_CRASH_REF_SOURCE").unwrap();
    let store = open(Path::new(&root));
    let target = PageId::from("pages/M target.md");
    let source = PageId::from(format!("pages/{source_name}.md"));
    let target_rev = store.read(&target.file(), None).unwrap().1;
    let source_rev = store.read(&source.file(), None).unwrap().1;
    let uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    let mut tx = store.page_plan(Some(crate::EditKind::RenamePage));
    tx.save_page(
        &[crate::EditKind::RenamePage],
        &target,
        SaveBase::Existing(target_rev),
        &page("M target", &format!("target\nid:: {uuid}")),
    );
    tx.trash(&source.file(), source_rev, crate::TrashIf::Any);
    let mut op = tx.plan().unwrap();
    op.moves.push((target, source));
    crate::host_faults::abort_before(crate::host_faults::Phase::TrashMove, 0);
    let host = crate::test_host_writer::HostWriter::start(&store);
    let _ = host.apply(op);
    panic!("the operation did not abort before the source's deletion");
}

/// The receiver is durable before its source goes, whichever name sorts first
/// (a process kill; `binding_order_tests` pins the order in process).
#[test]
fn a_receiver_is_durable_before_its_source_is_deleted_for_both_name_orders() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    for source_name in ["A source", "Z source"] {
        let root = scratch("block-reference");
        fs::write(root.join(format!("pages/{source_name}.md")), b"- draft\n").unwrap();
        fs::write(root.join("pages/M target.md"), b"- target\n").unwrap();
        let output = Command::new(tine_store::test_binary::own_test_binary())
            .arg("--exact")
            .arg("test_transactions::i02_crash_gaps::crash_receiver_before_source_worker")
            .arg("--nocapture")
            .env("TINE_CRASH_REF_ROOT", &root)
            .env("TINE_CRASH_REF_SOURCE", source_name)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "child must abort before the deletion"
        );
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("did not abort"),
            "{output:?}"
        );
        let reopened = open(&root);
        reopened.whole_graph().unwrap();
        assert_eq!(
            fs::read(root.join(format!("pages/{source_name}.md"))).unwrap(),
            b"- draft\n",
            "the source stays until its receiver is durable"
        );
        let target = fs::read_to_string(root.join("pages/M target.md")).unwrap();
        assert!(
            target.contains(&format!("id:: {uuid}")),
            "receiver missing after the kill"
        );
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn crash_rewritten_move_worker() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let Ok(root) = std::env::var("TINE_CRASH_MOVE_ROOT") else {
        return;
    };
    let point = match std::env::var("TINE_CRASH_MOVE_POINT").as_deref() {
        Ok("rename") => FaultPoint::AbortAfterMoveRename,
        Ok("rewrite") => FaultPoint::AbortAfterMoveRewrite,
        _ => return,
    };
    let store = open(Path::new(&root));
    let ext = std::env::var("TINE_CRASH_MOVE_EXT").unwrap_or_else(|_| "md".into());
    let source = store.file_id(Area::Assets, &format!("Old.{ext}")).unwrap();
    let destination = store.file_id(Area::Assets, &format!("New.{ext}")).unwrap();
    let rev = store.read(&source, None).unwrap().1;
    let mut tx = store.transaction(None);
    tx.move_file(
        &source,
        rev,
        &destination,
        Some(&RenameMap(vec![("Old".into(), "New".into())], Vec::new())),
    );
    store.inject_fault(point);
    let _ = tx.commit();
    panic!("rewritten move did not abort at injected boundary");
}

#[test]
fn rewritten_move_kill_reopen_has_one_live_page_at_each_internal_boundary() {
    let _integration_guard = crate::test_transactions::integration_lock();
    for (point, expected) in [
        ("rename", b"- [[Old]]\n".as_slice()),
        ("rewrite", b"- [[New]]\n".as_slice()),
    ] {
        let root = scratch("rewritten-move");
        fs::write(root.join("assets/Old.md"), b"- [[Old]]\n").unwrap();
        let output = Command::new(tine_store::test_binary::own_test_binary())
            .arg("--exact")
            .arg("test_transactions::i02_crash_gaps::crash_rewritten_move_worker")
            .arg("--nocapture")
            .env("TINE_CRASH_MOVE_ROOT", &root)
            .env("TINE_CRASH_MOVE_POINT", point)
            .output()
            .unwrap();
        assert!(!output.status.success(), "child must abort at {point}");
        for _ in 0..2 {
            let reopened = open(&root);
            reopened.whole_graph().unwrap();
            assert!(
                !root.join("assets/Old.md").exists(),
                "source must not reappear after {point}"
            );
            assert_eq!(fs::read(root.join("assets/New.md")).unwrap(), expected);
            drop(reopened);
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn failed_rewritten_move_removes_its_trash_copy_during_rollback() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let root = scratch("move-trash-rollback");
    let old = b"title:: Old\n- [[Old]]\n";
    fs::write(root.join("assets/Old.md"), old).unwrap();
    let store = open(&root);
    let source = store.file_id(Area::Assets, "Old.md").unwrap();
    let destination = store.file_id(Area::Assets, "New.md").unwrap();
    let rev = store.read(&source, None).unwrap().1;
    let mut tx = store.transaction(None);
    tx.move_file(
        &source,
        rev,
        &destination,
        Some(&RenameMap(vec![("Old".into(), "New".into())], Vec::new())),
    );
    store.inject_fault(FaultPoint::MoveAfterTrashCopyIo);
    assert!(matches!(tx.commit(), crate::TxOutcome::NotCommitted { .. }));
    assert_eq!(fs::read(root.join("assets/Old.md")).unwrap(), old);
    assert!(!root.join("assets/New.md").exists());
    let trash = root.join("logseq/.tine-trash");
    fn count_files(path: &Path) -> usize {
        fs::read_dir(path)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| {
                        if entry.path().is_dir() {
                            count_files(&entry.path())
                        } else {
                            1
                        }
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    assert_eq!(
        count_files(&trash),
        0,
        "failed move leaked a new trash copy"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
