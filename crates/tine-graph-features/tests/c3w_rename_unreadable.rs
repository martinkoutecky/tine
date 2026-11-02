//! C3W W2 (L03, I-2): a rename that must move a page it cannot read (non-UTF-8
//! bytes or an outline past the parse depth cap, e.g. delivered by a sync
//! service or an import) used to skip that page yet rewrite every referrer and
//! report Renamed: `[[New]]` then pointed at no file while the page stayed Old.
//! The rename must refuse, naming the file, and leave every byte in place.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages;
use tine_store::{Depth, Store};

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
    let _integration_guard = crate::integration_lock();
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

/// A block carrying this panics a reference-index stage (tine-store's
/// `TEST_INDEX_PANIC_SENTINEL`, on under `test-faults`): the page parses, and
/// its row is retired and listed unreadable.
const INDEX_PANIC: &str = "__TINE_TEST_INDEX_PANIC__";

/// `pages/Holder.md`, listed unreadable with `holder`'s bytes: from the load
/// when `at_load`, else by an index-stage panic after it.
fn listed_holder(label: &str, holder: &[u8], at_load: bool) -> (PathBuf, std::sync::Arc<Store>) {
    let (root, store) = fixture(label, b"- child\n");
    let holder_path = root.join("pages/Holder.md");
    fs::write(&holder_path, holder).unwrap();
    let store = if at_load {
        store.close();
        drop(store);
        std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0)
    } else {
        store
    };
    store.refresh(Depth::Bytes).unwrap();
    let view = store.whole_graph().unwrap();
    assert!(
        (view.unreadable_files().iter()).any(|(id, _)| id.as_str() == "pages/Holder.md"),
        "{label}: {:?}",
        view.unreadable_files()
    );
    (root, store)
}

/// The reference index has no row for a listed file, so `explicit_referrers`
/// omits it. A rename then left `[[Target]]` dangling in it with no word, and
/// repairing the page brought the broken link back. It refuses, naming the
/// file, before anything is written: Tine never rewrites a page it could not
/// index. A file too big or too deep for the rewriter refuses when it holds the
/// name.
#[test]
fn a_rename_refuses_when_a_listed_page_may_hold_the_name() {
    let _integration_guard = crate::integration_lock();
    let retired = format!("- {INDEX_PANIC} [[Target]]\n");
    let mut deep = deep_outline();
    deep.extend_from_slice(b"- the deep page links [[Target]]\n");
    let over_cap = |line: &[u8]| {
        let mut bytes = vec![b' '; tine_store::PARSE_INPUT_MAX_BYTES as usize + 1024 * 1024];
        // Across a chunk boundary of the search, which must still see it.
        let at = 3 * 1024 * 1024 + 24 - 13;
        bytes[at..at + line.len()].copy_from_slice(line);
        bytes
    };
    for (label, holder, at_load) in [
        ("retired", retired.as_bytes().to_vec(), false),
        ("non-utf8", b"- links [[Target]] \xff\xfe\n".to_vec(), true),
        ("too-deep", deep, true),
        ("over-cap", over_cap(b"- links [[target]]\n"), true),
    ] {
        let (root, store) = listed_holder(label, &holder, at_load);
        let before = snapshot(&root);
        let result = hosted(&store, |host| {
            pages::rename_or_merge_page(&store, host, "Target", "Renamed", None, None, &[])
        });
        let error = result.expect_err(label);
        assert!(
            error.to_string().contains("Holder.md"),
            "{label}: the refusal names the file: {error}"
        );
        assert_eq!(before, snapshot(&root), "{label}: a refusal writes nothing");
        let _ = fs::remove_dir_all(&root);
    }
}

/// A listed file that does not mention the renamed name blocks nothing, and
/// the rename leaves its bytes alone.
#[test]
fn a_rename_passes_a_listed_page_that_does_not_hold_the_name() {
    let _integration_guard = crate::integration_lock();
    let retired = format!("- {INDEX_PANIC} [[Elsewhere]]\n");
    let mut deep = deep_outline();
    deep.extend_from_slice(b"- the deep page says nothing of it\n");
    let mut over_cap = vec![b' '; tine_store::PARSE_INPUT_MAX_BYTES as usize + 1024];
    over_cap.extend_from_slice(b"- links [[Elsewhere]]\n");
    for (label, holder, at_load) in [
        ("too-deep-other", deep, true),
        ("over-cap-other", over_cap, true),
        ("retired-other", retired.as_bytes().to_vec(), false),
        (
            "non-utf8-other",
            b"- links [[Elsewhere]] \xff\xfe\n".to_vec(),
            true,
        ),
    ] {
        let (root, store) = listed_holder(label, &holder, at_load);
        let result = hosted(&store, |host| {
            pages::rename_or_merge_page(&store, host, "Target", "Renamed", None, None, &[])
        });
        assert!(result.is_ok(), "{label}: {result:?}");
        assert_eq!(fs::read(root.join("pages/Holder.md")).unwrap(), holder);
        assert!(fs::read_to_string(root.join("pages/Referrer.md"))
            .unwrap()
            .contains("[[Renamed]]"));
        let _ = fs::remove_dir_all(&root);
    }
}
