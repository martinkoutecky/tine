use std::{fs, path::PathBuf};
use tine_graph_features::assets;
use tine_store::{Area, Content, EditKind, Store, TxOutcome};
fn graph(tag: &str) -> (PathBuf, Store) {
    let root = std::env::temp_dir().join(format!("tine-fail2-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let (store, _, _) = Store::open(&root, Default::default()).unwrap();
    (root, store)
}
#[test]
fn config_failure_opens_but_never_creates_in_default_directories() {
    let (root, first) = graph("config");
    first.close();
    fs::create_dir_all(root.join("logseq/config.edn")).unwrap();
    let (store, _, config) = Store::open(&root, Default::default()).unwrap();
    assert!(config.problem.is_some());
    fs::write(root.join("pages/Existing.md"), "- still readable\n").unwrap();
    let existing = store.file_id(Area::Pages, "Existing.md").unwrap();
    assert!(store.page(&existing.as_str().into()).unwrap().doc.read_only);
    let id = store.file_id(Area::Pages, "Wrong.md").unwrap();
    let mut tx = store.transaction(Some(EditKind::CreatePage));
    tx.create(&id, Content::Bytes(b"- wrong\n".to_vec()));
    assert!(
        matches!(tx.commit(), TxOutcome::NotCommitted { .. }),
        "unknown configured directories must block writes"
    );
    assert!(!root.join("pages/Wrong.md").exists());
    fs::remove_dir(root.join("logseq/config.edn")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"notes\"}\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    assert!(store.config().problem.is_none());
    let id = store.file_id(Area::Pages, "Recovered.md").unwrap();
    let mut tx = store.transaction(Some(EditKind::CreatePage));
    tx.create(&id, Content::Bytes(b"- recovered\n".to_vec()));
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    assert!(root.join("notes/Recovered.md").exists());
    assert!(!root.join("pages/Recovered.md").exists());
    store.close();
    let _ = fs::remove_dir_all(root);
}
#[test]
fn discovery_read_failure_is_reported_and_good_pages_survive() {
    let (root, store) = graph("discovery");
    fs::write(root.join("pages/Good.md"), "- good\n").unwrap();
    fs::write(root.join("pages/Bad.md"), b"title:: \xff\n- bad\n").unwrap();
    store.scan_refresh().unwrap();
    let view = store.whole_graph().unwrap();
    assert!(view
        .unreadable_files()
        .iter()
        .any(|(id, _)| id.as_str() == "pages/Bad.md"));
    assert!(matches!(
        view.resolve("Good", false),
        tine_store::Resolved::Existing { .. }
    ));
    store.close();
    let _ = fs::remove_dir_all(root);
}
#[test]
fn trash_rechecks_references_after_an_external_publication() {
    let (root, store) = graph("trash");
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(root.join("assets/kept.png"), b"asset").unwrap();
    store.scan_refresh().unwrap();
    assert_eq!(assets::orphan_assets(&store).unwrap().len(), 1);
    fs::write(
        root.join("pages/Arrived.md"),
        "- ![kept](../assets/kept.png)\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    assert!(
        assets::trash_asset(&store, "kept.png").is_err(),
        "latest published references must defeat old orphan listing"
    );
    assert_eq!(fs::read(root.join("assets/kept.png")).unwrap(), b"asset");
    store.close();
    let _ = fs::remove_dir_all(root);
}
