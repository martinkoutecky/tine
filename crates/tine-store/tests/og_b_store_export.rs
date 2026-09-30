use std::fs;
use tine_store::Store;

#[test]
fn export_targets_use_real_ids_in_markdown_and_org() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::create_dir_all(dir.path().join("logseq")).unwrap();
    fs::write(
        dir.path().join("logseq/config.edn"),
        "{:publishing/all-pages-public? true}",
    )
    .unwrap();
    let md_id = "11111111-1111-4111-8111-111111111111";
    let org_id = "22222222-2222-4222-8222-222222222222";
    fs::write(dir.path().join("pages/Markdown.md"), format!(
        "- target\n  id:: {md_id}\n  ```\n  id:: 33333333-3333-4333-8333-333333333333\n  ```\n- (({org_id}))\n"
    )).unwrap();
    fs::write(
        dir.path().join("pages/Org.org"),
        format!("* target\n:PROPERTIES:\n:id: {org_id}\n:END:\n* (({md_id}))\n"),
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let corpus = store.whole_graph().unwrap().corpus();
    for (name, expected) in [("Markdown", md_id), ("Org", org_id)] {
        let page = corpus.pages.iter().find(|page| page.name == name).unwrap();
        assert_eq!(
            page.document.roots[0].property("id").as_deref(),
            Some(expected)
        );
    }
    tine_graph_features::publish::publish_html(&store).unwrap();
    let html: String = fs::read_dir(dir.path().join("publish"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    for id in [md_id, org_id] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "export anchor {id} missing"
        );
        assert!(
            html.contains(&format!(".html#{id}")),
            "export target {id} missing"
        );
    }
    store.close();
}
