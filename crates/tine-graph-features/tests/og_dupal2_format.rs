use std::fs;
use tine_graph_features::conflicts;
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

#[test]
fn uppercase_org_conflict_diff_keeps_outline_blocks_out_of_preamble() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("pages")).unwrap();
    let winner = "pages/Note.ORG";
    let copy = "pages/Note.sync-conflict-20261001-101010-ABCDEFG.ORG";
    fs::write(root.path().join(winner), "* mine\n** child\n").unwrap();
    fs::write(root.path().join(copy), "* theirs\n** child\n").unwrap();
    let store = std::sync::Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    let diff = conflicts::sync_conflict_diff(&store, winner, copy, &[])
        .unwrap()
        .unwrap();
    assert!(
        !diff.pre_differs,
        "Org headlines belong to outline rows, not the preamble"
    );
    assert!(
        !diff.rows.is_empty(),
        "Org conflict edits must appear as selectable rows"
    );
}

#[test]
fn uppercase_org_merge_keeps_source_header_and_outline() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("pages")).unwrap();
    fs::write(
        root.path().join("pages/Old.ORG"),
        "#+ALIAS: Former\n* source\n",
    )
    .unwrap();
    fs::write(root.path().join("pages/New.org"), "* survivor\n").unwrap();
    let store = std::sync::Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    hosted(&store, |host| {
        tine_graph_features::pages::rename_or_merge_page(
            &store,
            host,
            "Old",
            "New",
            None,
            Some("pages/New.org"),
            &[],
        )
    })
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("pages/New.org")).unwrap(),
        "#+ALIAS: Former\n* survivor\n* source\n"
    );
}
