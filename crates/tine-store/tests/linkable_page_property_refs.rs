use tine_store::{Depth, EditKind, PageId, QueryDialect, QueryResult, SaveBase, Store, TxOutcome};

#[test]
fn linkable_page_headers_match_queries_and_linked_references() {
    let _integration_guard = crate::integration_lock();
    for (extension, header) in [
        ("md", "related:: Alpha，Beta\nalias:: Editor\n\n- body\n"),
        ("org", "#+related: Alpha，Beta\n#+alias: Editor\n\n* body\n"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        std::fs::create_dir(dir.path().join("logseq")).unwrap();
        std::fs::write(
            dir.path().join("logseq/config.edn"),
            "{:property/separated-by-commas #{:related}}",
        )
        .unwrap();
        std::fs::write(dir.path().join(format!("pages/Source.{extension}")), header).unwrap();
        std::fs::write(dir.path().join("pages/Editor.md"), "- target\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        for target in ["Alpha", "Beta", "Editor"] {
            let graph = store.whole_graph().unwrap();
            let QueryResult::Simple(groups) = graph
                .query(&format!("[[{target}]]"), QueryDialect::Simple)
                .unwrap()
            else {
                panic!("simple rows")
            };
            let source = groups
                .iter()
                .find(|group| group.page == "Source")
                .unwrap_or_else(|| panic!("{extension}: query {target}: {groups:?}"));
            assert!(source.blocks[0].page_property);
            let linked = graph.backlinks(target).unwrap();
            assert_eq!(linked.len(), 1, "{extension}: linked {target}: {linked:?}");
            assert!(linked[0].blocks[0].page_property);
            assert!(!linked[0].evidence.is_empty());
        }
        store.close();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        assert_eq!(
            store
                .whole_graph()
                .unwrap()
                .backlinks("Beta")
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn property_references_refresh_after_guarded_saves_and_config_changes() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("logseq")).unwrap();
    let config_path = dir.path().join("logseq/config.edn");
    std::fs::write(&config_path, "{:property/separated-by-commas #{:related}}").unwrap();
    std::fs::write(
        dir.path().join("pages/Source.md"),
        "related:: Alpha，Beta\n\n- body\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    assert_eq!(
        store
            .whole_graph()
            .unwrap()
            .backlinks("Beta")
            .unwrap()
            .len(),
        1
    );
    // Prime an empty answer: adding its first referrer must evict it too.
    assert!(store
        .whole_graph()
        .unwrap()
        .backlinks("Delta")
        .unwrap()
        .is_empty());
    let id = PageId::from("pages/Source.md");
    let mut read = store.page(&id).unwrap();
    read.doc.pre_block = Some("related:: Delta".into());
    let mut tx = store.transaction(Some(EditKind::SaveBlock));
    tx.save_page(
        &[EditKind::SaveBlock],
        &id,
        SaveBase::Existing(read.rev),
        &read.doc,
    );
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    let graph = store.whole_graph().unwrap();
    assert!(graph.backlinks("Beta").unwrap().is_empty());
    assert_eq!(graph.backlinks("Delta").unwrap().len(), 1);
    let QueryResult::Simple(groups) = graph.query("[[Delta]]", QueryDialect::Simple).unwrap()
    else {
        panic!("simple rows")
    };
    assert_eq!(groups.len(), 1);
    drop(graph);
    std::fs::write(&config_path, "{}").unwrap();
    store.refresh(Depth::Bytes).unwrap();
    assert!(store
        .whole_graph()
        .unwrap()
        .backlinks("Delta")
        .unwrap()
        .is_empty());
    std::fs::write(&config_path, "{:property/separated-by-commas #{:related}}").unwrap();
    store.refresh(Depth::Bytes).unwrap();
    assert_eq!(
        store
            .whole_graph()
            .unwrap()
            .backlinks("Delta")
            .unwrap()
            .len(),
        1
    );
}
