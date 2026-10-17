use tine_store::{QueryDialect, Store};

#[test]
fn org_headline_tags_are_page_refs() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("pages/Source.org"),
        "* the plugins :Plugins:Équipe:\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    for name in ["plugins", "équipe"] {
        let tine_store::QueryResult::Simple(groups) = graph
            .query(&format!("[[{name}]]"), QueryDialect::Simple)
            .unwrap()
        else {
            panic!("simple query must return block groups");
        };
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].blocks[0].raw, "the plugins :Plugins:Équipe:");
        let refs = graph.backlinks(name).unwrap();
        assert_eq!(refs.len(), 1, "{refs:?}");
        assert_eq!(refs[0].blocks[0].raw, "the plugins :Plugins:Équipe:");
        assert!(!refs[0].evidence.is_empty());
    }
    drop(graph);
    drop(store);
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    assert_eq!(
        store
            .whole_graph()
            .unwrap()
            .backlinks("plugins")
            .unwrap()
            .len(),
        1
    );
}
