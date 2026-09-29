//! C3W W3 (L03): rename rewrites every reference to the old name that OG's
//! rename rewrites. An unmatched `[[` (also `#[[`) used to swallow text up to the
//! next `]]`, so `[[Target]]` after it stayed stale; a nested `[[a [[Target]] c]]`
//! lost its inner reference the same way; and a `tags::` value split with the
//! full-width `，` (which the reference evidence counts) was never rewritten.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages;
use tine_store::Store;

fn fixture(files: &[(&str, &str)]) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3w-w3-{}-{}",
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
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    (root, store)
}

#[test]
fn w3_rename_rewrites_references_after_an_unmatched_opener_and_fullwidth_tags() {
    let (root, store) = fixture(&[
        ("pages/Target.md", "- the target\n"),
        (
            "pages/Referrer.md",
            "- use [[ to link, then [[Target]] here\n\
             - a tag #[[ opener then #[[Target]] too\n\
             - nested [[a [[Target]] c]]\n\
             - tagged\n  tags:: Other，Target\n\
             - plain [[Target]]\n",
        ),
        (
            "pages/Org.org",
            "* see [[ x and [[file:./Target.org][Target]] and [[Target]]\n",
        ),
    ]);
    pages::rename_or_merge_page(&store, "Target", "Renamed", None, None, &[]).unwrap();
    assert!(root.join("pages/Renamed.md").exists());
    let referrer = fs::read_to_string(root.join("pages/Referrer.md")).unwrap();
    assert_eq!(
        referrer,
        "- use [[ to link, then [[Renamed]] here\n\
         - a tag #[[ opener then #Renamed too\n\
         - nested [[a [[Renamed]] c]]\n\
         - tagged\n  tags:: Other，Renamed\n\
         - plain [[Renamed]]\n"
    );
    let org = fs::read_to_string(root.join("pages/Org.org")).unwrap();
    assert!(
        org.contains("[[file:./Renamed.org][Target]]") && org.contains("and [[Renamed]]"),
        "{org}"
    );
    let _ = fs::remove_dir_all(&root);
}
