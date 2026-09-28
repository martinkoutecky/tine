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
