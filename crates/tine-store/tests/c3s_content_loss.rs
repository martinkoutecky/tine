//! Checkpoint-3 S findings (content loss), each driven through the public
//! `Store` entry points a user action reaches.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_store::{PageId, SaveBase, SaveOutcome, Store};

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3s-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    root
}

fn save_raw(store: &Store, id: &PageId, raw: &[&str]) -> SaveOutcome {
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
    store.save(
        tine_store::EditKind::ReplacePage,
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
        let store = Store::open(&root, Default::default()).unwrap().0;
        // Wait for the background warm: the finding needs the warm page cache.
        store.whole_graph().unwrap();
        let id = PageId::from(rel.as_str());
        assert_eq!(raws(&store, &id), ["a"], "{format}");

        // Own save, then reload: the editor must see what it just saved.
        assert!(
            matches!(save_raw(&store, &id, &["b"]), SaveOutcome::Saved(_)),
            "{format}"
        );
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

/// F2 (L07): a multi-page save whose later entry fails is undone by
/// withdrawing each written file's new bytes to conflict trash and then
/// rewriting its old bytes. A crash between the two must still leave every
/// page's pre-save bytes on disk: live, or recoverable in graph trash.
#[cfg(feature = "test-faults")]
#[test]
fn f2_crash_in_multi_page_undo_keeps_each_pages_old_bytes() {
    let root = scratch("f2");
    fs::write(root.join("pages/A.md"), "- old A\n").unwrap();
    fs::write(root.join("pages/B.md"), "- old B\n").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "f2_undo_crash_worker", "--nocapture"])
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
    let live = fs::read_to_string(root.join("pages/A.md")).ok();
    assert!(
        matches!(live.as_deref(), Some("- old A\n" | "- new A\n"))
            || exists_anywhere(&root.join("logseq/.tine-trash"), b"- old A\n")
            || exists_anywhere(&root.join("assets/.tine-restore-recovery"), b"- old A\n"),
        "I-2: page A's pre-save bytes are on disk nowhere after a crash in undo \
         (live {live:?}); exemplar: the rewritten move's trash copy in Transaction::apply"
    );
    // B's racing external write is untouched.
    assert_eq!(
        fs::read_to_string(root.join("pages/B.md")).unwrap(),
        "external stage-2"
    );
    reopened.close();
    let _ = fs::remove_dir_all(&root);
}

/// F2 neighbour: an uninterrupted rollback restores the old bytes and withdraws
/// its staged `tx-old` copy, so a refused multi-page save leaves no trash debris.
#[cfg(feature = "test-faults")]
#[test]
fn f2_completed_undo_restores_old_bytes_and_leaves_no_staged_copy() {
    let root = scratch("f2-ok");
    fs::write(root.join("pages/A.md"), "- old A\n").unwrap();
    fs::write(root.join("pages/B.md"), "- old B\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let entries = two_page_entries(&store);
    store.inject_fault(tine_store::FaultPoint::Stage2MismatchAt(1));
    assert!(matches!(
        store.save_pages(&entries),
        tine_store::SavePagesOutcome::Failed { index: 1, .. }
    ));
    assert_eq!(
        fs::read_to_string(root.join("pages/A.md")).unwrap(),
        "- old A\n"
    );
    assert!(!exists_anywhere(
        &root.join("logseq/.tine-trash"),
        b"- old A\n"
    ));
    store.close();
    let _ = fs::remove_dir_all(&root);
}

#[cfg(feature = "test-faults")]
fn two_page_entries(
    store: &Store,
) -> Vec<(
    PageId,
    SaveBase,
    tine_core::model::PageDto,
    Vec<tine_store::EditKind>,
)> {
    ["A", "B"]
        .into_iter()
        .map(|name| {
            let id = PageId::from(format!("pages/{name}.md").as_str());
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = format!("new {name}");
            (
                id,
                SaveBase::Existing(read.rev),
                doc,
                vec![tine_store::EditKind::ReplacePage],
            )
        })
        .collect()
}

#[cfg(feature = "test-faults")]
#[test]
fn f2_undo_crash_worker() {
    let Ok(root) = std::env::var("TINE_C3S_ROOT") else {
        return;
    };
    let store = Store::open(Path::new(&root), Default::default()).unwrap().0;
    let entries = two_page_entries(&store);
    // An external writer changes B just before its write (an external-editor
    // race): B is refused unwritten and undo runs for A alone.
    store.inject_fault(tine_store::FaultPoint::Stage2MismatchAt(1));
    store.inject_fault(tine_store::FaultPoint::AbortAfterUndoWithdraw);
    let _ = store.save_pages(&entries);
    panic!("undo did not reach the withdraw abort point");
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
