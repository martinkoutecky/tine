//! og C3Y Y4: `alias::` members are separated by `,` or the full-width `，`
//! (OG graph-parser `text.cljs` `sep-by-comma`, `#"[\,，]{1}"`), the answer
//! C3W W3 gave the reference evidence and the `tags::` rename. Rename-merge's
//! alias union split on `,` only, so a survivor's `A，Shared` hid `Shared`
//! and the merge wrote a duplicate alias into the survivor's header.
use std::fs;
use std::path::PathBuf;
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

fn fixture(files: &[(&str, &str)]) -> (PathBuf, std::sync::Arc<Store>) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3y-alias-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "assets"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    (root, store)
}

#[test]
fn y4_merge_unites_full_width_separated_aliases_without_duplicates() {
    let (root, store) = fixture(&[
        ("pages/Old.md", "alias:: Shared，Former\n\n- source block\n"),
        ("pages/New.md", "alias:: Kept，Shared\n\n- survivor block\n"),
    ]);
    hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Old", "New", None, Some("pages/New.md"), &[])
    })
    .unwrap();
    let merged = fs::read_to_string(root.join("pages/New.md")).unwrap();
    assert!(
        merged.starts_with("alias:: Kept，Shared, Former\n"),
        "each alias once, the survivor's line kept verbatim: {merged}"
    );
    let graph = store.whole_graph().unwrap();
    for name in ["New", "Former", "Kept", "Shared"] {
        match graph.resolve(name, false) {
            tine_store::Resolved::Existing { id, .. } => assert_eq!(id.as_str(), "pages/New.md"),
            tine_store::Resolved::Alias { owners } => {
                assert_eq!(owners[0].as_str(), "pages/New.md", "{name}")
            }
            tine_store::Resolved::Absent { .. } => panic!("{name} lost its page"),
        }
    }
    let _ = fs::remove_dir_all(&root);
}

/// OG `rename-update-refs!` rewrites a referrer's content only through
/// `replace-old-page!` (literal `[[Old]]`, `#Old`, an `Old::` key), so a bare
/// `alias::` member naming the renamed page stays; a bracketed one follows.
#[test]
fn y4_rename_keeps_a_bare_alias_member_and_rewrites_a_bracketed_one() {
    let (root, store) = fixture(&[
        ("pages/Old.md", "- old body\n"),
        ("pages/Other.md", "- other\n  alias:: Old，[[Old]]\n"),
        ("pages/Ref.md", "- [[Old]]\n"),
    ]);
    hosted(&store, |host| {
        pages::rename_page_expected(&store, host, "Old", "New", None)
    })
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("pages/Other.md")).unwrap(),
        "- other\n  alias:: Old，[[New]]\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
        "- [[New]]\n"
    );
    let _ = fs::remove_dir_all(&root);
}
