use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages;
use tine_store::Store;

fn fixture(label: &str) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-client-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}
fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn title_owned_rename_rebinds_identity_and_plain_rename_keeps_lookup() {
    for (label, physical, source) in [
        (
            "title",
            "Physical",
            "title:: Effective\n\n- [[Effective]] body\n",
        ),
        (
            "title-matched",
            "Effective",
            "title:: Effective\n\n- [[Effective]] body\n",
        ),
        ("plain", "Physical", "- [[Physical]] body\n"),
    ] {
        let (root, _) = fixture(&format!("rename-identity-{label}"));
        put(&root, &format!("pages/{physical}.md"), source);
        put(
            &root,
            "pages/Ref.md",
            if label != "plain" {
                "- [[Effective]]\n"
            } else {
                "- [[Physical]]\n"
            },
        );
        let store = Store::open(&root, Default::default()).unwrap().0;
        let old = if label != "plain" {
            "Effective"
        } else {
            "Physical"
        };
        pages::rename_page_expected(
            &store,
            old,
            "Renamed",
            Some(&format!("pages/{physical}.md")),
        )
        .unwrap();
        assert!(!root.join(format!("pages/{physical}.md")).exists());
        let moved = root.join("pages/Renamed.md");
        assert!(moved.exists());
        let bytes = fs::read_to_string(&moved).unwrap();
        if label != "plain" {
            assert_eq!(bytes, "title:: Renamed\n\n- [[Renamed]] body\n");
        } else {
            assert_eq!(bytes, "- [[Renamed]] body\n");
        }
        assert_eq!(
            fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
            "- [[Renamed]]\n"
        );
        let graph = store.whole_graph().unwrap();
        assert!(matches!(
            graph.resolve("Renamed", false),
            tine_store::Resolved::Existing { .. }
        ));
        assert!(matches!(
            graph.resolve(old, false),
            tine_store::Resolved::Absent { .. }
        ));
    }
}

/// Batch 11 follow-up: an Org page named by its `#+TITLE:` directive (either
/// case) or a `:title:` drawer property is renamed like a Markdown `title::`
/// page. The directive keeps its own spelling; only the name changes. Master
/// leaves the old directive, so the moved page keeps its old name and every
/// rewritten `[[Renamed]]` dangles (receipt 12B, differential).
#[test]
fn org_title_directive_rename_rebinds_identity() {
    for (label, before, after) in [
        (
            "upper",
            "#+TITLE: Effective\n\n* [[Effective]] body\n",
            "#+TITLE: Renamed\n\n* [[Renamed]] body\n",
        ),
        (
            "lower",
            "#+title:  Effective\r\n* [[Effective]] body\r\n",
            "#+title:  Renamed\r\n* [[Renamed]] body\r\n",
        ),
        (
            "drawer",
            ":PROPERTIES:\n:title: Effective\n:END:\n* [[Effective]] body\n",
            ":PROPERTIES:\n:title: Renamed\n:END:\n* [[Renamed]] body\n",
        ),
        (
            "custom",
            "#+TITLE: Effective\n#+AUTHOR: me\n* body\n",
            "#+TITLE: Renamed\n#+AUTHOR: me\n* body\n",
        ),
    ] {
        for physical in ["Physical", "Effective"] {
            let (root, _) = fixture(&format!("rename-org-title-{label}"));
            let source = format!("pages/{physical}.org");
            put(&root, &source, before);
            put(&root, "pages/Ref.md", "- [[Effective]]\n");
            let store = Store::open(&root, Default::default()).unwrap().0;
            pages::rename_page_expected(&store, "Effective", "Renamed", Some(&source)).unwrap();
            assert!(!root.join(&source).exists(), "{label}");
            assert_eq!(
                fs::read_to_string(root.join("pages/Renamed.org")).unwrap(),
                after,
                "{label}"
            );
            assert_eq!(
                fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
                "- [[Renamed]]\n"
            );
            let graph = store.whole_graph().unwrap();
            assert!(
                matches!(
                    graph.resolve("Renamed", false),
                    tine_store::Resolved::Existing { .. }
                ),
                "{label}"
            );
            assert!(
                matches!(
                    graph.resolve("Effective", false),
                    tine_store::Resolved::Absent { .. }
                ),
                "{label}"
            );
        }
    }
}

/// An Org title that names something other than the page being moved is user
/// content: a rename of a different page never rewrites it.
#[test]
fn org_title_naming_another_page_is_left_alone() {
    let (root, _) = fixture("rename-org-title-other");
    put(
        &root,
        "pages/Other.org",
        "#+TITLE: Other\n* see [[Effective]]\n",
    );
    put(&root, "pages/Effective.md", "- body\n");
    let store = Store::open(&root, Default::default()).unwrap().0;
    pages::rename_page_expected(&store, "Effective", "Renamed", None).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("pages/Other.org")).unwrap(),
        "#+TITLE: Other\n* see [[Renamed]]\n"
    );
}

/// An imported Org page Tine cannot reproduce byte-for-byte keeps its bytes:
/// rebinding its `#+TITLE:` would rewrite it, so the rename refuses and
/// nothing moves (storage contract, `rewrite_move::ReadOnly`).
#[test]
fn org_title_rename_refuses_a_page_that_does_not_round_trip() {
    let (root, _) = fixture("rename-org-title-readonly");
    let source = "#+TITLE: Effective\n* Parent\n*** child\n";
    put(&root, "pages/Physical.org", source);
    let store = Store::open(&root, Default::default()).unwrap().0;
    let error = pages::rename_page_expected(&store, "Effective", "Renamed", None)
        .expect_err("a non-round-tripping Org title must not be rewritten");
    assert!(error.to_string().contains("round-trip"), "{error}");
    assert_eq!(
        fs::read_to_string(root.join("pages/Physical.org")).unwrap(),
        source
    );
    assert!(!root.join("pages/Renamed.org").exists());
}
