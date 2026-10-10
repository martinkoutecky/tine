//! C3W W2 (L03, I-2): a rename that must move a page it cannot read (non-UTF-8
//! bytes or an outline past the parse depth cap, e.g. delivered by a sync
//! service or an import) used to skip that page yet rewrite every referrer and
//! report Renamed: `[[New]]` then pointed at no file while the page stayed Old.
//! The rename must refuse, naming the file, and leave every byte in place.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages;
use tine_store::Store;

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

fn fixture(label: &str, child: &[u8]) -> (PathBuf, std::sync::Arc<Store>) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3w-w2-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    fs::write(root.join("pages/Target.md"), "- the target\n").unwrap();
    fs::write(root.join("pages/Target___Child.md"), child).unwrap();
    fs::write(
        root.join("pages/Referrer.md"),
        "- links [[Target]] and [[Target/Child]]\n",
    )
    .unwrap();
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (root, store)
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root.join("pages")).unwrap().flatten() {
        files.push((entry.path(), fs::read(entry.path()).unwrap()));
    }
    files.sort();
    files
}

fn deep_outline() -> Vec<u8> {
    let mut text = String::new();
    for depth in 0..200 {
        text.push_str(&"  ".repeat(depth));
        text.push_str("- d\n");
    }
    text.into_bytes()
}

#[test]
fn w2_rename_refuses_when_a_page_it_must_move_is_unreadable() {
    for (label, child) in [
        ("non-utf8", b"- child \xff\xfe\n".to_vec()),
        ("too-deep", deep_outline()),
    ] {
        let (root, store) = fixture(label, &child);
        let before = snapshot(&root);
        let result = hosted(&store, |host| {
            pages::rename_or_merge_page(&store, host, "Target", "Renamed", None, None, &[])
        });
        let after = snapshot(&root);
        match result {
            Err(error) => {
                assert!(
                    error.to_string().contains("Target___Child.md"),
                    "{label}: refusal must name the file: {error}"
                );
                assert_eq!(before, after, "{label}: a refused rename writes nothing");
            }
            Ok(report) => {
                // Acceptable only if the unreadable page moved with its name.
                let moved = root.join("pages/Renamed___Child.md");
                assert!(
                    moved.exists() && fs::read(&moved).unwrap() == child,
                    "{label}: rename reported {report:?} but the unreadable page did not move: {after:?}"
                );
            }
        }
        let _ = fs::remove_dir_all(&root);
    }
}
