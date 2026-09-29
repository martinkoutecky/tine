#![cfg(feature = "test-faults")]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_graph_features::{conflicts, pages};
use tine_store::{Area, FaultPoint, OpenOptions, PageId, RenameMap, RestoreFile, SaveBase, Store};

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

fn open(root: &Path) -> Store {
    Store::open(root, OpenOptions::default()).unwrap().0
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
    let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.save_page(
        &[tine_store::EditKind::ReplacePage],
        &a,
        SaveBase::Existing(a_rev),
        &page("A", "new A"),
    );
    tx.move_file(&b, b_rev, &d, None);
    tx.trash(&c, c_rev);
    store.inject_fault(FaultPoint::AbortAfterStep(boundary.parse().unwrap()));
    let _ = tx.commit();
    panic!("I-2: transaction fault did not abort; exemplar Transaction::commit");
}

#[test]
fn transaction_kill_reopen_preserves_each_old_or_new_file() {
    for boundary in 0..3 {
        let root = scratch("tx");
        fs::write(root.join("pages/A.md"), b"- old A\n").unwrap();
        fs::write(root.join("pages/B.md"), b"- old B\n").unwrap();
        fs::write(root.join("assets/c.bin"), b"old C").unwrap();
        child(&root, "crash_transaction_worker", boundary);
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
    let mut tx = store.transaction(Some(tine_store::EditKind::SaveBlock));
    tx.save_page(
        &[tine_store::EditKind::SaveBlock],
        &target,
        SaveBase::Existing(target_rev),
        &page("M target", &format!("target\nid:: {uuid}")),
    );
    tx.save_page(
        &[tine_store::EditKind::SaveBlock],
        &source,
        SaveBase::Existing(source_rev),
        &page(&source_name, &format!("(({uuid}))")),
    );
    store.inject_fault(FaultPoint::AbortAfterStep(0));
    let _ = tx.commit();
    panic!("block-reference transaction did not abort");
}

#[test]
fn block_reference_target_is_durable_at_the_step_boundary_for_both_name_orders() {
    let uuid = "48ae2a7a-e09b-4a21-aa3a-010101010101";
    for source_name in ["A source", "Z source"] {
        let root = scratch("block-reference");
        fs::write(root.join(format!("pages/{source_name}.md")), b"- draft\n").unwrap();
        fs::write(root.join("pages/M target.md"), b"- target\n").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("crash_block_reference_worker")
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
    let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.move_file(
        &source,
        rev,
        &destination,
        Some(&RenameMap(vec![("Old".into(), "New".into())])),
    );
    store.inject_fault(point);
    let _ = tx.commit();
    panic!("rewritten move did not abort at injected boundary");
}

#[test]
fn rewritten_move_kill_reopen_has_one_live_page_at_each_internal_boundary() {
    for (point, expected) in [
        ("rename", b"- [[Old]]\n".as_slice()),
        ("rewrite", b"- [[New]]\n".as_slice()),
    ] {
        let root = scratch("rewritten-move");
        fs::write(root.join("pages/Old.md"), b"- [[Old]]\n").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("crash_rewritten_move_worker")
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
fn rewritten_move_with_preamble_title_retries_after_rename_crash() {
    for (ext, old, new_title) in [
        ("md", "title:: Old\n- [[Old]]\n", "title:: New"),
        ("org", "#+TITLE: Old\n* [[Old]]\n", "#+TITLE: New"),
    ] {
        let root = scratch("move-title-retry");
        fs::write(root.join(format!("pages/Old.{ext}")), old).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("crash_rewritten_move_worker")
            .env("TINE_CRASH_MOVE_ROOT", &root)
            .env("TINE_CRASH_MOVE_EXT", ext)
            .env("TINE_CRASH_MOVE_POINT", "rename")
            .output()
            .unwrap();
        assert!(!output.status.success(), "child must abort after rename");
        let reopened = open(&root);
        reopened.whole_graph().unwrap();
        assert!(!root.join(format!("pages/Old.{ext}")).exists());
        assert_eq!(
            fs::read_to_string(root.join(format!("pages/New.{ext}"))).unwrap(),
            old
        );
        pages::rename_page_expected(&reopened, "Old", "New", None).unwrap();
        let final_bytes = fs::read_to_string(root.join(format!("pages/New.{ext}"))).unwrap();
        assert!(
            final_bytes.contains(new_title),
            "title not rebound: {final_bytes:?}"
        );
        assert!(
            final_bytes.contains("[[New]]"),
            "self-reference not rebound: {final_bytes:?}"
        );
        assert!(!root.join(format!("pages/Old.{ext}")).exists());
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn failed_rewritten_move_removes_its_trash_copy_during_rollback() {
    let root = scratch("move-trash-rollback");
    let old = b"title:: Old\n- [[Old]]\n";
    fs::write(root.join("pages/Old.md"), old).unwrap();
    let store = open(&root);
    let source = store.file_id(Area::Pages, "Old.md").unwrap();
    let destination = store.file_id(Area::Pages, "New.md").unwrap();
    let rev = store.read(&source, None).unwrap().1;
    let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
    tx.move_file(
        &source,
        rev,
        &destination,
        Some(&RenameMap(vec![("Old".into(), "New".into())])),
    );
    store.inject_fault(FaultPoint::MoveAfterTrashCopyIo);
    assert!(matches!(
        tx.commit(),
        tine_store::TxOutcome::NotCommitted { .. }
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

#[test]
fn crash_restore_worker() {
    if std::env::var("TINE_CRASH_KIND").as_deref() != Ok("restore") {
        return;
    }
    let root = PathBuf::from(std::env::var("TINE_CRASH_ROOT").unwrap());
    let store = open(&root);
    let files = ["A.md", "B.md"]
        .into_iter()
        .map(|rel| {
            let source = File::open(root.join("snapshot").join(rel)).unwrap();
            let len = source.metadata().unwrap().len();
            RestoreFile {
                area: Area::Pages,
                rel: rel.into(),
                source,
                len,
            }
        })
        .collect();
    let _ = store.restore(tine_store::EditKind::ReplacePage, files, None);
    panic!("I-2: restore fault did not abort; exemplar Store::restore");
}

#[test]
fn restore_kill_reopen_keeps_live_or_recovery_bytes() {
    for boundary in 0..4 {
        let root = scratch("restore");
        fs::create_dir_all(root.join("snapshot")).unwrap();
        for name in ["A", "B"] {
            fs::write(root.join(format!("pages/{name}.md")), format!("old {name}")).unwrap();
            fs::write(
                root.join(format!("snapshot/{name}.md")),
                format!("new {name}"),
            )
            .unwrap();
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("crash_restore_worker")
            .arg("--nocapture")
            .env("TINE_CRASH_KIND", "restore")
            .env("TINE_CRASH_ROOT", &root)
            .env("TINE_RESTORE_ABORT_BOUNDARY", boundary.to_string())
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "I-2: restore child must abort at boundary {boundary}; exemplar Store::restore"
        );
        let reopened = open(&root);
        reopened.whole_graph().unwrap();
        for name in ["A", "B"] {
            let old = format!("old {name}");
            let new = format!("new {name}");
            let live = fs::read(root.join(format!("pages/{name}.md"))).ok();
            assert!(live.as_deref() == Some(old.as_bytes()) || live.as_deref() == Some(new.as_bytes()) || live.is_none(),
                "I-2: restore left torn bytes for {name} at boundary {boundary}; exemplar Store::restore");
            assert!(
                live.as_deref() == Some(old.as_bytes()) || recovery_has(&root, old.as_bytes()),
                "I-2: restore lost old {name} at boundary {boundary}; exemplar Store::restore"
            );
        }
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn crash_graph_restore_worker() {
    if std::env::var("TINE_CRASH_KIND").as_deref() != Ok("graph-restore") {
        return;
    }
    let root = PathBuf::from(std::env::var("TINE_CRASH_ROOT").unwrap());
    let store = open(&root);
    let files = ["pages/A.md", "Root.md"]
        .into_iter()
        .map(|rel| {
            let source = File::open(root.join("snapshot").join(rel)).unwrap();
            let len = source.metadata().unwrap().len();
            RestoreFile {
                area: Area::Graph,
                rel: rel.into(),
                source,
                len,
            }
        })
        .collect();
    let _ = store.restore(tine_store::EditKind::ReplacePage, files, Some(&[]));
    panic!("I-2: graph restore fault did not abort; exemplar Store::restore");
}

/// og-B whole-graph restore: text outside `pages/` and `journals/` (a root
/// page, a stale page under `archive/`) crosses the same durable boundaries.
/// A kill at any of them leaves each file whole, live or in recovery.
#[test]
fn graph_restore_kill_reopen_keeps_live_or_recovery_bytes() {
    for boundary in 0..5 {
        let root = scratch("graph-restore");
        fs::create_dir_all(root.join("snapshot/pages")).unwrap();
        fs::create_dir_all(root.join("archive")).unwrap();
        for rel in ["pages/A.md", "Root.md"] {
            fs::write(root.join(rel), format!("old {rel}")).unwrap();
            fs::write(root.join("snapshot").join(rel), format!("new {rel}")).unwrap();
        }
        fs::write(root.join("archive/Stale.md"), "stale").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("crash_graph_restore_worker")
            .arg("--nocapture")
            .env("TINE_CRASH_KIND", "graph-restore")
            .env("TINE_CRASH_ROOT", &root)
            .env("TINE_RESTORE_ABORT_BOUNDARY", boundary.to_string())
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "I-2: graph restore child must abort at boundary {boundary}; exemplar Store::restore"
        );
        let reopened = open(&root);
        reopened.whole_graph().unwrap();
        for rel in ["pages/A.md", "Root.md"] {
            let old = format!("old {rel}");
            let new = format!("new {rel}");
            let live = fs::read(root.join(rel)).ok();
            assert!(live.as_deref() == Some(old.as_bytes()) || live.as_deref() == Some(new.as_bytes()) || live.is_none(),
                "I-2: graph restore left torn bytes for {rel} at boundary {boundary}; exemplar Store::restore");
            assert!(
                live.as_deref() == Some(old.as_bytes()) || recovery_has(&root, old.as_bytes()),
                "I-2: graph restore lost old {rel} at boundary {boundary}; exemplar Store::restore"
            );
        }
        assert!(
            fs::read(root.join("archive/Stale.md")).is_ok_and(|bytes| bytes == b"stale")
                || recovery_has(&root, b"stale"),
            "I-2: graph restore lost a retired page at boundary {boundary}; exemplar Store::restore"
        );
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn crash_feature_worker() {
    let Ok(journey) = std::env::var("TINE_CRASH_JOURNEY") else {
        return;
    };
    let root = PathBuf::from(std::env::var("TINE_CRASH_ROOT").unwrap());
    let boundary: usize = std::env::var("TINE_CRASH_BOUNDARY")
        .unwrap()
        .parse()
        .unwrap();
    let store = open(&root);
    store.whole_graph().unwrap();
    store.inject_fault(FaultPoint::AbortAfterStep(boundary));
    match journey.as_str() {
        "merge" => {
            pages::merge_pages(&store, "pages/src.md", "pages/dst.md").unwrap();
        }
        "rename" => {
            pages::rename_page_expected(&store, "A", "B", None).unwrap();
        }
        "rename-merge" => {
            pages::rename_or_merge_page(&store, "Old", "New", None, Some("pages/New.md"), &[])
                .unwrap();
        }
        "conflict" => {
            let copy = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";
            let diff = conflicts::sync_conflict_diff(&store, "pages/Foo.md", copy, &[])
                .unwrap()
                .unwrap();
            let decisions = diff
                .rows
                .iter()
                .map(|row| (row.id.clone(), "both".to_owned()))
                .collect();
            conflicts::resolve_sync_conflict(
                &store,
                "pages/Foo.md",
                copy,
                &decisions,
                &diff.base_rev,
                &diff.conflict_rev,
                None,
                &[],
                "union",
            )
            .unwrap();
        }
        _ => panic!("unknown crash journey"),
    }
    panic!("I-2: feature fault did not abort; exemplar Transaction::commit");
}

const RENAME_MERGE_SOURCE: &[u8] = b"type:: note\n\n- moved source [[Old/Kid]]\n";

/// I-2 for `pages::rename_or_merge_page` at one crash boundary, and after the
/// same merge is retried (`done`): no source block twice in the survivor,
/// `[[Old]]`/`[[New]]` always resolve to a live page, and every source byte is
/// in the survivor or in trash. A descendant ref may name `New/Kid` before
/// its move lands (the plain rename's window); it converges on retry.
fn rename_merge_boundary_holds(root: &Path, boundary: usize, done: bool) {
    let at = format!("boundary {boundary}, retried {done}; exemplar pages::rename_or_merge_page");
    let merged = fs::read_to_string(root.join("pages/New.md")).unwrap();
    assert!(
        merged.contains("kept destination"),
        "I-2: survivor kept at {at}"
    );
    for needle in ["moved source", "type:: note"] {
        assert!(
            merged.matches(needle).count() <= 1,
            "I-2: rename-merge duplicated {needle:?} in the survivor at {at}:\n{merged}"
        );
    }
    let source_live =
        fs::read(root.join("pages/Old.md")).ok().as_deref() == Some(RENAME_MERGE_SOURCE);
    assert!(
        source_live
            || (merged.contains("moved source [[New/Kid]]")
                && recovery_has(root, RENAME_MERGE_SOURCE)),
        "I-2: rename-merge source bytes must be live or merged and in trash at {at}:\n{merged}"
    );
    let reference = fs::read_to_string(root.join("pages/Ref.md")).unwrap();
    assert!(
        reference == "- [[Old]] reference [[Old/Kid]]\n"
            || reference == "- [[New]] reference [[New/Kid]]\n",
        "I-2: rename-merge reference rewrite must be whole old/new bytes at {at}"
    );
    assert!(
        !reference.contains("[[Old]]") || source_live,
        "I-2: [[Old]] must resolve to the live source until its referrers are rewritten at {at}"
    );
    let kid = fs::read(root.join("pages/Old___Kid.md"))
        .ok()
        .or_else(|| fs::read(root.join("pages/New___Kid.md")).ok());
    assert_eq!(
        kid.as_deref(),
        Some(&b"- kid body\n"[..]),
        "I-2: descendant kept at {at}"
    );
    if done {
        assert_eq!(merged.matches("moved source").count(), 1,
            "I-2: retry must leave the source blocks in the survivor exactly once at {at}:\n{merged}");
        assert!(
            !root.join("pages/Old.md").exists() && recovery_has(root, RENAME_MERGE_SOURCE),
            "I-2: retry must finish with the source in trash at {at}"
        );
        assert_eq!(
            reference, "- [[New]] reference [[New/Kid]]\n",
            "I-2: retry must finish referrers at {at}"
        );
        assert!(
            root.join("pages/New___Kid.md").exists(),
            "I-2: retry must finish the descendant move at {at}"
        );
    }
}

#[test]
fn feature_journeys_kill_reopen_keep_content() {
    for journey in ["merge", "rename", "rename-merge", "conflict"] {
        // rename-merge steps: save survivor, rewrite Ref.md, move Old/Kid, trash Old.
        for boundary in 0..if journey == "rename-merge" { 4 } else { 2 } {
            let root = scratch(journey);
            match journey {
                "merge" => {
                    fs::write(root.join("pages/src.md"), b"- moved source\n").unwrap();
                    fs::write(root.join("pages/dst.md"), b"- kept destination\n").unwrap();
                }
                "rename" => {
                    fs::write(root.join("pages/A.md"), b"- original page\n").unwrap();
                    fs::write(root.join("pages/Ref.md"), b"- [[A]] reference\n").unwrap();
                }
                "rename-merge" => {
                    fs::write(
                        root.join("logseq/config.edn"),
                        b"{:file/name-format :triple-lowbar}\n",
                    )
                    .unwrap();
                    fs::write(root.join("pages/Old.md"), RENAME_MERGE_SOURCE).unwrap();
                    fs::write(root.join("pages/Old___Kid.md"), b"- kid body\n").unwrap();
                    fs::write(root.join("pages/New.md"), b"- kept destination\n").unwrap();
                    fs::write(
                        root.join("pages/Ref.md"),
                        b"- [[Old]] reference [[Old/Kid]]\n",
                    )
                    .unwrap();
                }
                "conflict" => {
                    fs::write(root.join("pages/Foo.md"), b"- mine content\n").unwrap();
                    fs::write(
                        root.join("pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md"),
                        b"- theirs content\n",
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let output = Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("crash_feature_worker")
                .arg("--nocapture")
                .env("TINE_CRASH_JOURNEY", journey)
                .env("TINE_CRASH_ROOT", &root)
                .env("TINE_CRASH_BOUNDARY", boundary.to_string())
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "I-2: {journey} must abort after step {boundary}; exemplar Transaction::commit: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            let reopened = open(&root);
            reopened.whole_graph().unwrap();
            match journey {
                "merge" => {
                    let merged = fs::read_to_string(root.join("pages/dst.md")).unwrap();
                    assert!(merged.contains("kept destination") && merged.contains("moved source"),
                        "I-2: merge must keep both blocks at step {boundary}; exemplar pages::merge_pages");
                    assert!(root.join("pages/src.md").exists() || recovery_has(&root, b"- moved source\n"),
                        "I-2: source must remain live or recoverable at step {boundary}; exemplar pages::merge_pages");
                }
                "rename-merge" => {
                    rename_merge_boundary_holds(&root, boundary, false);
                    if root.join("pages/Old.md").exists() {
                        pages::rename_or_merge_page(
                            &reopened,
                            "Old",
                            "New",
                            None,
                            Some("pages/New.md"),
                            &[],
                        )
                        .unwrap();
                    }
                    rename_merge_boundary_holds(&root, boundary, true);
                }
                "rename" => {
                    let moved = fs::read(root.join("pages/B.md")).ok();
                    let old = fs::read(root.join("pages/A.md")).ok();
                    assert!(moved.as_deref() == Some(b"- original page\n") || old.as_deref() == Some(b"- original page\n"),
                        "I-2: rename must keep source bytes at step {boundary}; exemplar pages::rename_page_expected");
                    let reference = fs::read_to_string(root.join("pages/Ref.md")).unwrap();
                    assert!(reference == "- [[A]] reference\n" || reference == "- [[B]] reference\n",
                        "I-2: reference rewrite must be whole old/new bytes at step {boundary}; exemplar pages::rename_page_expected");
                }
                "conflict" => {
                    let winner = fs::read_to_string(root.join("pages/Foo.md")).unwrap();
                    assert!(winner.contains("mine content") && winner.contains("theirs content"),
                        "I-2: resolved winner must contain both sides at step {boundary}; exemplar conflicts::resolve_sync_conflict");
                    assert!(root.join("pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md").exists()
                        || recovery_has(&root, b"- theirs content\n"),
                        "I-2: conflict copy must remain live or recoverable at step {boundary}; exemplar conflicts::resolve_sync_conflict");
                }
                _ => unreachable!(),
            }
            drop(reopened);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn durable_transition_inventory_names_crash_proofs() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (file, owner, proof) in [
        (
            "crates/tine-store/src/transaction.rs",
            "pub fn commit",
            "AbortAfterStep(index)",
        ),
        (
            "crates/tine-store/src/transaction.rs",
            "fn undo<'b>",
            "FaultPoint::AbortAfterUndoWithdraw",
        ),
        (
            "crates/tine-store/src/restore.rs",
            "pub fn restore",
            "restore_abort_boundary()",
        ),
        (
            "src-tauri/src/backup.rs",
            "fn do_backup_source_cancellable",
            "publish_snapshot(&dest",
        ),
        (
            "crates/tine-store/src/publish.rs",
            "commit_publish_stage_report",
            "previous_kept",
        ),
    ] {
        let source = fs::read_to_string(repo.join(file)).unwrap();
        assert!(source.contains(owner) && source.contains(proof),
            "I-2: every multi-step durable transition needs a gap case or named recovery; exemplar transaction.rs:1365 Transaction::commit; missing {file} {owner} {proof}");
    }
}
