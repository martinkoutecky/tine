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
    store.refresh(tine_store::Depth::Stamps).unwrap();
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
    store.refresh(tine_store::Depth::Stamps).unwrap();
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
    store.refresh(tine_store::Depth::Stamps).unwrap();
    assert_eq!(assets::orphan_assets(&store).unwrap().len(), 1);
    fs::write(
        root.join("pages/Arrived.md"),
        "- ![kept](../assets/kept.png)\n",
    )
    .unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    assert_eq!(
        assets::trash_asset(&store, "kept.png").unwrap(),
        assets::TrashOutcome::Referenced,
        "latest published references must defeat old orphan listing"
    );
    assert_eq!(fs::read(root.join("assets/kept.png")).unwrap(), b"asset");
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cold_title_discovery_failure_never_blocks_other_names() {
    let (root, old) = graph("cold-title");
    old.close();
    fs::write(root.join("pages/Bad.md"), b"title:: Claimed \xff\n- bad\n").unwrap();
    let (store, _, _) = Store::open(&root, Default::default()).unwrap();
    let missing = store.page_named("Claimed", tine_core::model::PageKind::Page);
    let view = store.whole_graph().unwrap();
    store.close();
    assert!(
        matches!(missing, Ok(None)),
        "a miss stays a miss while another file is unreadable"
    );
    assert!(
        view.unreadable_files()
            .iter()
            .any(|(id, _)| id.as_str() == "pages/Bad.md"),
        "an undecodable name is reported, not silently absent"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn journal_scan_failure_never_reports_a_completed_empty_feed() {
    let (root, store) = graph("journal-scan");
    store.close();
    let error = tine_graph_features::journals::feed_page(&store, 10, None)
        .err()
        .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn journal_preview_failure_is_reported_instead_of_an_empty_preview() {
    let (root, store) = graph("journal-preview");
    fs::write(root.join("journals/2026_06_19.md"), "- good\n").unwrap();
    fs::write(root.join("journals/2026_06_19.org"), b"- bad \xff\n").unwrap();
    // I-22: the bad file stays listed with its failure, never an empty
    // preview that reads as an empty file, and never hiding the day.
    let days = tine_graph_features::journals::journal_conflicts(&store).unwrap();
    let bad = days
        .iter()
        .flat_map(|day| &day.files)
        .find(|file| file.path == "journals/2026_06_19.org")
        .expect("the unreadable duplicate stays listed");
    assert!(
        bad.preview_error.is_some(),
        "incomplete duplicate journal previews must be reported"
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn oversized_css_and_directory_read_failure_are_not_css_absence() {
    let (root, store) = graph("css-errors");
    fs::create_dir_all(root.join("logseq")).unwrap();
    let path = root.join("logseq/custom.css");
    let file = fs::File::create(&path).unwrap();
    file.set_len(tine_store::PARSE_INPUT_MAX_BYTES + 1).unwrap();
    assert_eq!(
        tine_graph_features::config::custom_css(&store)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
    drop(file);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(tine_graph_features::config::custom_css(&store).is_err());
    fs::remove_dir(&path).unwrap();
    assert_eq!(tine_graph_features::config::custom_css(&store).unwrap(), "");
    store.close();
    let _ = fs::remove_dir_all(root);
}
