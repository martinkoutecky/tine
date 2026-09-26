#![cfg(feature = "test-faults")]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_graph_features::{conflicts, pages};
use tine_store::{Area, FaultPoint, OpenOptions, PageId, RestoreFile, SaveBase, Store};

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
    let mut tx = store.transaction();
    tx.save_page(&a, SaveBase::Existing(a_rev), &page("A", "new A"));
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
    let _ = store.restore(files);
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
        "conflict" => {
            let copy = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";
            let diff = conflicts::sync_conflict_diff(&store, "pages/Foo.md", copy)
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
                "union",
            )
            .unwrap();
        }
        _ => panic!("unknown crash journey"),
    }
    panic!("I-2: feature fault did not abort; exemplar Transaction::commit");
}

#[test]
fn feature_journeys_kill_reopen_keep_content() {
    for journey in ["merge", "rename", "conflict"] {
        for boundary in 0..2 {
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
