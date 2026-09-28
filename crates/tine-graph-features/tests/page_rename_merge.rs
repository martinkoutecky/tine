//! Family 16: rename onto an existing page merges (master 7160c501cb88, GH #327;
//! OG `merge-pages!`), and namespaced `title::` pages rebind on rename (GH #451).
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages;
use tine_store::Store;

fn fixture(label: &str, files: &[(&str, &str)]) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-rename-merge-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, body) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

const LOWBAR: &str = "{:file/name-format :triple-lowbar}\n";

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

fn trashed(root: &Path, bytes: &str) -> bool {
    fn walk(dir: &Path, bytes: &str) -> bool {
        fs::read_dir(dir).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, bytes)
                } else {
                    fs::read_to_string(path).is_ok_and(|found| found == bytes)
                }
            })
        })
    }
    walk(&root.join("logseq/.tine-trash"), bytes)
}

/// Master `rename_collision_merges_content_and_rewrites_graph_refs`, in meaning.
#[test]
fn rename_collision_merges_content_and_rewrites_graph_refs() {
    let (root, store) = fixture(
        "master",
        &[
            ("logseq/config.edn", LOWBAR),
            (
                "pages/Old.md",
                "- old body links [[Old]] and [[Old/Child]]\n",
            ),
            ("pages/New.md", "- new body links [[Old]]\n"),
            ("pages/Old___Child.md", "- child of [[Old]]\n"),
            ("pages/Referrer.md", "- see [[Old]] and #Old\n"),
        ],
    );
    let plain = pages::rename_page_expected(&store, "Old", "New", Some("pages/Old.md"));
    assert_eq!(
        plain.unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists,
        "an unconfirmed rename onto an existing page still refuses"
    );
    assert_eq!(read(&root, "pages/New.md"), "- new body links [[Old]]\n");

    pages::rename_or_merge_page(
        &store,
        "Old",
        "New",
        Some("pages/Old.md"),
        Some("pages/New.md"),
    )
    .unwrap();

    let merged = read(&root, "pages/New.md");
    assert!(merged.contains("new body links [[New]]"), "{merged}");
    assert!(
        merged.contains("old body links [[New]] and [[New/Child]]"),
        "{merged}"
    );
    assert_eq!(read(&root, "pages/Referrer.md"), "- see [[New]] and #New\n");
    assert_eq!(read(&root, "pages/New___Child.md"), "- child of [[New]]\n");
    assert!(!root.join("pages/Old.md").exists());
    assert!(!root.join("pages/Old___Child.md").exists());
    assert!(
        trashed(&root, "- old body links [[Old]] and [[Old/Child]]\n"),
        "the merged source stays recoverable in graph trash"
    );
    let graph = store.whole_graph().unwrap();
    assert!(matches!(
        graph.resolve("Old", false),
        tine_store::Resolved::Absent { .. }
    ));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// OG `merge-pages!` moves the source's property block with its blocks; the
/// source's identity (`title::`) never renames the survivor, and aliases join.
#[test]
fn rename_merge_keeps_survivor_identity_and_joins_aliases() {
    let (root, store) = fixture(
        "aliases",
        &[
            (
                "pages/Old.md",
                "title:: Old\nalias:: Former, Shared\nicon:: star\n\n- source block\n",
            ),
            ("pages/New.md", "alias:: Shared, Kept\n\n- survivor block\n"),
            ("pages/Ref.md", "- [[Former]] and [[Kept]] and [[Old]]\n"),
        ],
    );
    pages::rename_or_merge_page(&store, "Old", "New", None, Some("pages/New.md")).unwrap();
    let merged = read(&root, "pages/New.md");
    assert!(
        merged.starts_with("alias:: Shared, Kept, Former\nicon:: star\n"),
        "{merged}"
    );
    assert!(merged.contains("- survivor block\n"), "{merged}");
    assert!(merged.contains("- source block\n"), "{merged}");
    assert!(
        merged.contains("title:: Old"),
        "the source title survives as ordinary block text: {merged}"
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
    assert_eq!(
        read(&root, "pages/Ref.md"),
        "- [[Former]] and [[Kept]] and [[New]]\n"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rename_merge_refuses_before_writing_when_its_confirmation_is_stale() {
    let files = [
        ("logseq/config.edn", LOWBAR),
        ("pages/Old.md", "- old\n"),
        ("pages/New.md", "- new\n"),
        ("pages/Other.md", "- other\n"),
        ("pages/Old___Kid.md", "- kid\n"),
        ("pages/New___Kid.md", "- existing kid\n"),
        ("pages/Ref.md", "- [[Old]]\n"),
    ];
    let (root, store) = fixture("stale", &files);
    // The confirmed survivor is not the page that now owns the name.
    let wrong = pages::rename_or_merge_page(&store, "Old", "New", None, Some("pages/Other.md"));
    assert!(wrong.is_err());
    // A namespace child whose target exists refuses the whole operation.
    let child = pages::rename_or_merge_page(&store, "Old", "New", None, Some("pages/New.md"));
    assert_eq!(child.unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    for (rel, body) in files {
        assert_eq!(read(&root, rel), body, "{rel} must be untouched");
    }
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rename_merge_refuses_mixed_formats() {
    let (root, store) = fixture(
        "formats",
        &[("pages/Old.org", "* old\n"), ("pages/New.md", "- new\n")],
    );
    let result = pages::rename_or_merge_page(&store, "Old", "New", None, Some("pages/New.md"));
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(read(&root, "pages/Old.org"), "* old\n");
    assert_eq!(read(&root, "pages/New.md"), "- new\n");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// GH #451: a namespaced page whose own `title::` names it keeps its identity
/// through a parent rename, for the page and its descendants.
#[test]
fn namespaced_title_pages_rebind_their_own_title_on_rename() {
    let (root, store) = fixture(
        "ns-title",
        &[
            ("pages/proj.md", "title:: Proj\n\n- parent\n"),
            (
                "pages/proj___sub.md",
                "title:: Proj/Sub\n\n- child [[Proj]]\n",
            ),
            ("pages/Loose.md", "title:: Proj/Loose\n\n- elsewhere\n"),
            ("pages/Ref.md", "- [[Proj/Sub]] [[Proj/Loose]]\n"),
        ],
    );
    pages::rename_page_expected(&store, "Proj", "Arch", Some("pages/proj.md")).unwrap();
    assert_eq!(read(&root, "pages/Arch.md"), "title:: Arch\n\n- parent\n");
    assert_eq!(
        read(&root, "pages/Arch%2FSub.md"),
        "title:: Arch/Sub\n\n- child [[Arch]]\n"
    );
    assert_eq!(
        read(&root, "pages/Arch%2FLoose.md"),
        "title:: Arch/Loose\n\n- elsewhere\n"
    );
    assert_eq!(
        read(&root, "pages/Ref.md"),
        "- [[Arch/Sub]] [[Arch/Loose]]\n"
    );
    let graph = store.whole_graph().unwrap();
    for name in ["Arch", "Arch/Sub", "Arch/Loose"] {
        assert!(
            matches!(
                graph.resolve(name, false),
                tine_store::Resolved::Existing { .. }
            ),
            "{name}"
        );
    }
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// A plain merge never lets the source's `title::` rename the survivor: with
/// no `title::` of its own, the survivor used to take it and change identity.
#[test]
fn plain_merge_never_carries_the_source_title_into_the_survivor_header() {
    let (root, store) = fixture(
        "plain-title",
        &[
            ("pages/src.md", "title:: Source\nnote:: kept\n\n- moved\n"),
            ("pages/dst.md", "- kept\n"),
        ],
    );
    pages::merge_pages(&store, "pages/src.md", "pages/dst.md").unwrap();
    let merged = read(&root, "pages/dst.md");
    assert!(merged.starts_with("note:: kept\n"), "{merged}");
    assert!(merged.contains("- title:: Source"), "{merged}");
    let graph = store.whole_graph().unwrap();
    assert!(matches!(
        graph.resolve("dst", false),
        tine_store::Resolved::Existing { .. }
    ));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
