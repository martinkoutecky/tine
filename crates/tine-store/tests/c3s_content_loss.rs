//! Checkpoint-3 S findings (content loss), each driven through the public
//! `Store` entry points a user action reaches.
use std::fs;
use std::path::{Path, PathBuf};

use crate::test_host_writer::{HostWriter, SaveRefused};
use crate::{FileRev, PageId, SaveBase, Store};
use std::sync::Arc;

fn scratch(label: &str) -> PathBuf {
    let root = crate::test_support::fresh_dir(&format!("tine-c3s-{label}-"));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    root
}

fn save_raw(store: &Arc<Store>, id: &PageId, raw: &[&str]) -> Result<FileRev, SaveRefused> {
    let read = store.page(id).unwrap();
    let mut doc = read.doc;
    let template = doc.blocks[0].clone();
    doc.blocks = raw
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let mut block = template.clone();
            block.id = format!("c3s-{i}");
            block.raw = (*text).into();
            block
        })
        .collect();
    HostWriter::for_store(store).save(
        crate::EditKind::ReplacePage,
        id,
        SaveBase::Existing(read.rev),
        &doc,
    )
}

fn raws(store: &Store, id: &PageId) -> Vec<String> {
    store
        .page(id)
        .unwrap()
        .doc
        .blocks
        .into_iter()
        .map(|b| b.raw)
        .collect()
}

/// F1 (L05): a journal whose configured `:journal/file-name-format` stem is not
/// `yyyy_MM_dd`/`yyyy-MM-dd` must never be treated as a "shadow" of itself. If it
/// is, the warm cache is never reconciled with the file, a reload serves the stale
/// cached text under the NEW disk revision, and the next save silently replaces
/// the newer bytes (own save, and an external/Syncthing write alike).
#[test]
fn f1_custom_journal_format_reload_serves_disk_and_never_clobbers_newer_bytes() {
    let _integration_guard = crate::test_transactions::integration_lock();
    for (format, stem) in [
        ("dd-MM-yyyy", "24-06-2026"),
        ("yyyyMMdd", "20260624"),
        ("yyyy.MM.dd", "2026.06.24"),
        ("MM-dd-yyyy", "06-24-2026"),
    ] {
        let root = scratch("f1");
        fs::write(
            root.join("logseq/config.edn"),
            format!("{{:journal/file-name-format \"{format}\"}}\n"),
        )
        .unwrap();
        let rel = format!("journals/{stem}.md");
        let path = root.join(&rel);
        fs::write(&path, "- a\n").unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        // Wait for the background warm: the finding needs the warm page cache.
        store.whole_graph().unwrap();
        let id = PageId::from(rel.as_str());
        assert_eq!(raws(&store, &id), ["a"], "{format}");

        // Own save, then reload: the editor must see what it just saved.
        assert!(save_raw(&store, &id, &["b"]).is_ok(), "{format}");
        // A save from that reload must keep `b` (it does not when the reload
        // served stale `a` under `b`'s revision).
        let reloaded = raws(&store, &id);
        let mut next: Vec<&str> = reloaded.iter().map(String::as_str).collect();
        next.push("c");
        let _ = save_raw(&store, &id, &next);
        let disk = fs::read_to_string(&path).unwrap();
        assert!(
            disk.contains("- b"),
            "{format}: own saved `b` was overwritten by a save from a stale reload \
             ({reloaded:?}): {disk:?}"
        );
        assert_eq!(reloaded, ["b"], "{format}: reload after own save");

        // External (sync/editor) write, then reload + save on its revision.
        fs::write(&path, "- x\n").unwrap();
        // The page is held since the save: its host hears of the write from
        // the watcher, and answers the read from then on.
        assert!(crate::test_host_writer::HostWriter::for_store(&store)
            .quiesce_for_tests(std::time::Duration::from_secs(20)));
        assert_eq!(
            raws(&store, &id),
            ["x"],
            "{format}: reload after external write"
        );
        let _ = save_raw(&store, &id, &["x", "c"]);
        let disk = fs::read_to_string(&path).unwrap();
        assert!(
            disk.contains("- x"),
            "{format}: the external write was overwritten: {disk:?}"
        );
        store.close();
        let _ = fs::remove_dir_all(&root);
    }
}

/// F2 (L07): a transaction whose later step fails is undone by withdrawing
/// each written file's new bytes to conflict trash and then rewriting its old
/// bytes. A crash between the two must still leave every file's earlier bytes
/// on disk: live, or recoverable in graph trash. Non-page files: the page host
/// promises no multi-page rollback (`page_operation_failures_retain_input_and_fresh_indexes_through_relaunch`).
#[cfg(feature = "test-faults")]
#[test]
fn f2_crash_in_multi_file_undo_keeps_each_files_old_bytes() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let root = scratch("f2");
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets/A.txt"), "- old A\n").unwrap();
    fs::write(root.join("assets/B.txt"), "- old B\n").unwrap();
    let output = std::process::Command::new(tine_store::test_binary::own_test_binary())
        .args([
            "--exact",
            "test_transactions::c3s_content_loss::f2_undo_crash_worker",
            "--nocapture",
        ])
        .env("TINE_C3S_ROOT", &root)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "worker must abort inside undo: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let reopened = Store::open(&root, Default::default()).unwrap().0;
    reopened.whole_graph().unwrap();
    // A was written and then withdrawn by undo; the crash came before its old
    // bytes were rewritten. Its content must be live (old or new) or its old
    // bytes recoverable in graph trash / recovery roots.
    let live = fs::read_to_string(root.join("assets/A.txt")).ok();
    assert!(
        matches!(live.as_deref(), Some("- old A\n" | "- new A\n"))
            || exists_anywhere(&root.join("logseq/.tine-trash"), b"- old A\n")
            || exists_anywhere(&root.join("assets/.tine-restore-recovery"), b"- old A\n"),
        "I-2: A's earlier bytes are on disk nowhere after a crash in undo \
         (live {live:?}); exemplar: the rewritten move's trash copy in Transaction::apply"
    );
    // B's racing external write is untouched.
    assert_eq!(
        fs::read_to_string(root.join("assets/B.txt")).unwrap(),
        "external stage-2"
    );
    reopened.close();
    let _ = fs::remove_dir_all(&root);
}

/// F2 neighbour: an uninterrupted rollback restores the old bytes and withdraws
/// its staged `tx-old` copy, so a refused transaction leaves no trash debris.
#[cfg(feature = "test-faults")]
#[test]
fn f2_completed_undo_restores_old_bytes_and_leaves_no_staged_copy() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let root = scratch("f2-ok");
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets/A.txt"), "- old A\n").unwrap();
    fs::write(root.join("assets/B.txt"), "- old B\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.inject_fault(crate::FaultPoint::Stage2MismatchAt(1));
    assert!(matches!(
        replace_both(&store),
        crate::TxOutcome::NotCommitted { step: 1, .. }
    ));
    assert_eq!(
        fs::read_to_string(root.join("assets/A.txt")).unwrap(),
        "- old A\n"
    );
    assert!(!exists_anywhere(
        &root.join("logseq/.tine-trash"),
        b"- old A\n"
    ));
    store.close();
    let _ = fs::remove_dir_all(&root);
}

/// One transaction replacing both assets, in order.
#[cfg(feature = "test-faults")]
fn replace_both(store: &Store) -> crate::TxOutcome {
    let mut tx = store.transaction(None);
    for name in ["A", "B"] {
        let file = store
            .file_id(crate::Area::Assets, &format!("{name}.txt"))
            .unwrap();
        let rev = store.read(&file, None).unwrap().1;
        tx.replace(&file, rev, format!("- new {name}\n").into_bytes());
    }
    tx.commit()
}

#[cfg(feature = "test-faults")]
#[test]
fn f2_undo_crash_worker() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let Ok(root) = std::env::var("TINE_C3S_ROOT") else {
        return;
    };
    let store = Store::open(Path::new(&root), Default::default()).unwrap().0;
    // An external writer changes B just before its write (an external-editor
    // race): B is refused unwritten and undo runs for A alone.
    store.inject_fault(crate::FaultPoint::Stage2MismatchAt(1));
    store.inject_fault(crate::FaultPoint::AbortAfterUndoWithdraw);
    let _ = replace_both(&store);
    panic!("undo did not reach the withdraw abort point");
}

/// F3 (L14) reachability: a file line with two spaces after the bullet keeps the
/// extra space in the block raw, so the editor's marker edits see `" TODO …"`
/// (pinned by `src/editor/leadingMarkerSplice.test.ts`).
#[test]
fn f3_extra_space_after_bullet_stays_in_the_block_raw() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let root = scratch("f3");
    fs::write(root.join("pages/T.md"), "-  TODO buy milk\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        raws(&store, &PageId::from("pages/T.md")),
        [" TODO buy milk"]
    );
    store.close();
    let _ = fs::remove_dir_all(&root);
}

fn exists_anywhere(root: &Path, needle: &[u8]) -> bool {
    fn walk(dir: &Path, needle: &[u8]) -> bool {
        let Ok(entries) = fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, needle)
            } else {
                fs::read(&path)
                    .is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
            }
        })
    }
    walk(root, needle)
}
