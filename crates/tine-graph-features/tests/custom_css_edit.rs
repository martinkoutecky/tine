//! Settings -> Theme "Edit custom.css": GH #610. The path handed to the OS opener
//! comes from the store; a missing stylesheet is created through one guarded
//! no-replace transaction, and an existing one is never read or rewritten.

use tine_graph_features::{config, custom_css};
use tine_store::{OpenOptions, Store};

fn graph(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "tine-css-edit-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for dir in ["pages", "journals"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn open(root: &std::path::Path) -> Store {
    Store::open(root, OpenOptions::default()).unwrap().0
}

#[test]
fn a_missing_custom_css_is_created_commented_and_then_left_alone() {
    let root = graph("create");
    let store = open(&root);
    assert!(!root.join("logseq/custom.css").exists());

    let path = custom_css::ensure_custom_css(&store).unwrap();
    assert_eq!(
        path,
        std::fs::canonicalize(root.join("logseq/custom.css")).unwrap()
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("/*") && text.trim_end().ends_with("*/"),
        "{text}"
    );
    assert!(
        text.contains("--tine-embed-bg"),
        "the starter names a real token"
    );
    // The created file is an ordinary stylesheet read by the one answerer.
    assert_eq!(config::custom_css(&store).unwrap(), text);

    std::fs::write(&path, "a { color: red }\n").unwrap();
    assert_eq!(custom_css::ensure_custom_css(&store).unwrap(), path);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a { color: red }\n",
        "an existing stylesheet must never be rewritten"
    );
    store.close();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_oversized_existing_custom_css_can_still_be_opened_for_repair() {
    let root = graph("oversized");
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    let big = vec![b'a'; 70 * 1024 * 1024];
    std::fs::write(root.join("logseq/custom.css"), &big).unwrap();
    let store = open(&root);
    assert!(config::custom_css(&store).is_err(), "the reader refuses it");
    assert!(
        custom_css::ensure_custom_css(&store).is_ok(),
        "the editor must not"
    );
    assert_eq!(
        std::fs::metadata(root.join("logseq/custom.css"))
            .unwrap()
            .len(),
        big.len() as u64
    );
    store.close();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_directory_named_custom_css_is_refused_not_replaced() {
    let root = graph("dir");
    std::fs::create_dir_all(root.join("logseq/custom.css")).unwrap();
    let store = open(&root);
    assert!(custom_css::ensure_custom_css(&store).is_err());
    assert!(root.join("logseq/custom.css").is_dir());
    store.close();
    std::fs::remove_dir_all(root).unwrap();
}
