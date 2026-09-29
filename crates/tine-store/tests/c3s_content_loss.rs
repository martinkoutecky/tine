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

#[allow(dead_code)]
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
