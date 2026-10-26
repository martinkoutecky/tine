use crate::{Area, FaultPoint, OpenOptions, PageId, RenameMap, SaveBase, Store};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
static SEQ: AtomicU64 = AtomicU64::new(0);

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "tine-i02-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
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
    let output = Command::new(std::env::current_exe().unwrap())
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
    let a = PageId::from("pages/A.md");
    let b = store.file_id(Area::Pages, "B.md").unwrap();
    let d = store.file_id(Area::Pages, "D.md").unwrap();
    let c = store.file_id(Area::Assets, "c.bin").unwrap();
    let a_rev = store.read(&a.file(), None).unwrap().1;
    let b_rev = store.read(&b, None).unwrap().1;
    let c_rev = store.read(&c, None).unwrap().1;
    let mut tx = store.page_plan(Some(crate::EditKind::ReplacePage));
    tx.save_page(
        &[crate::EditKind::ReplacePage],
        &a,
        SaveBase::Existing(a_rev),
        &page("A", "new A"),
    );
    tx.move_file(&b, b_rev, &d, None);
    tx.trash(&c, c_rev, crate::TrashIf::Any);
    store.inject_fault(FaultPoint::AbortAfterStep(boundary.parse().unwrap()));
    let _ = tx.commit_internal();
    panic!("I-2: transaction fault did not abort; exemplar Transaction::commit");
}

#[test]
fn transaction_kill_reopen_preserves_each_old_or_new_file() {
    let _integration_guard = crate::test_transactions::integration_lock();
    for boundary in 0..3 {
        let root = scratch("tx");
        fs::write(root.join("pages/A.md"), b"- old A\n").unwrap();
        fs::write(root.join("pages/B.md"), b"- old B\n").unwrap();
        fs::write(root.join("assets/c.bin"), b"old C").unwrap();
        child(
            &root,
            "test_transactions::i02_crash_gaps::crash_transaction_worker",
            boundary,
        );
        let reopened = open(&root);
        reopened.whole_graph().unwrap();
        let a = fs::read(root.join("pages/A.md")).unwrap();
        assert!(a == b"- old A\n" || a == b"- new A\n",
            "I-2: no torn page after commit crash; exemplar Transaction::commit at {boundary}: {a:?}");
        let b = fs::read(root.join("pages/B.md")).ok();
        let d = fs::read(root.join("pages/D.md")).ok();
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
fn crash_block_reference_worker() {
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
    let mut tx = store.page_plan(Some(crate::EditKind::SaveBlock));
    tx.save_page(
        &[crate::EditKind::SaveBlock],
        &target,
        SaveBase::Existing(target_rev),
        &page("M target", &format!("target\nid:: {uuid}")),
    );
    tx.save_page(
        &[crate::EditKind::SaveBlock],
        &source,
        SaveBase::Existing(source_rev),
        &page(&source_name, &format!("(({uuid}))")),
    );
    store.inject_fault(FaultPoint::AbortAfterStep(0));
    let _ = tx.commit_internal();
    panic!("block-reference transaction did not abort");
}

#[test]
fn block_reference_target_is_durable_at_the_step_boundary_for_both_name_orders() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    for source_name in ["A source", "Z source"] {
        let root = scratch("block-reference");
        fs::write(root.join(format!("pages/{source_name}.md")), b"- draft\n").unwrap();
        fs::write(root.join("pages/M target.md"), b"- target\n").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("test_transactions::i02_crash_gaps::crash_block_reference_worker")
            .env("TINE_CRASH_REF_ROOT", &root)
            .env("TINE_CRASH_REF_SOURCE", source_name)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "child must abort at the first save step"
        );
        let reopened = open(&root);
        reopened.whole_graph().unwrap();
        let source = fs::read_to_string(root.join(format!("pages/{source_name}.md"))).unwrap();
        let target = fs::read_to_string(root.join("pages/M target.md")).unwrap();
        assert!(
            !source.contains(&format!("(({uuid}))")),
            "source reference reached disk first"
        );
        assert!(
            target.contains(&format!("id:: {uuid}")),
            "target ID missing after first step"
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
    let source = store.file_id(Area::Pages, &format!("Old.{ext}")).unwrap();
    let destination = store.file_id(Area::Pages, &format!("New.{ext}")).unwrap();
    let rev = store.read(&source, None).unwrap().1;
    let mut tx = store.page_plan(Some(crate::EditKind::ReplacePage));
    tx.move_file(
        &source,
        rev,
        &destination,
        Some(&RenameMap(vec![("Old".into(), "New".into())])),
    );
    store.inject_fault(point);
    let _ = tx.commit_internal();
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
        fs::write(root.join("pages/Old.md"), b"- [[Old]]\n").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
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
                !root.join("pages/Old.md").exists(),
                "source must not reappear after {point}"
            );
            assert_eq!(fs::read(root.join("pages/New.md")).unwrap(), expected);
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
    fs::write(root.join("pages/Old.md"), old).unwrap();
    let store = open(&root);
    let source = store.file_id(Area::Pages, "Old.md").unwrap();
    let destination = store.file_id(Area::Pages, "New.md").unwrap();
    let rev = store.read(&source, None).unwrap().1;
    let mut tx = store.page_plan(Some(crate::EditKind::RenamePage));
    tx.move_file(
        &source,
        rev,
        &destination,
        Some(&RenameMap(vec![("Old".into(), "New".into())])),
    );
    store.inject_fault(FaultPoint::MoveAfterTrashCopyIo);
    assert!(matches!(
        tx.commit_internal(),
        crate::TxOutcome::NotCommitted { .. }
    ));
    assert_eq!(fs::read(root.join("pages/Old.md")).unwrap(), old);
    assert!(!root.join("pages/New.md").exists());
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
