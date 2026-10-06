//! Family 22 (master 0d45ffc6b): the page named by `:tine/favorites-page` is a
//! sidebar arrangement, never a reference source. A page merely CALLED
//! Favorites, without the key, stays an ordinary page.
use std::fs;
use tine_store::Store;

fn names(groups: &[tine_core::model::RefGroup]) -> Vec<String> {
    let mut names = groups.iter().map(|g| g.page.clone()).collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn favorites_layout_page_is_never_a_reference_source() {
    let root = std::env::temp_dir().join(format!("tine-fav-exclude-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("pages/Notes.md"),
        "- a real mention [[Target]] and Target\n",
    )
    .unwrap();
    fs::write(
        root.join("pages/Favorites.md"),
        "tine/favorites:: true\n\n- [[Target]]\n- Work Target\n\t- [[Target]]\n",
    )
    .unwrap();
    fs::write(root.join("logseq/config.edn"), "{}\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    assert_eq!(
        names(&graph.backlinks("Target").unwrap()),
        vec!["Favorites".to_string(), "Notes".to_string()],
        "an unmarked page named Favorites is just a page"
    );
    drop(graph);
    fs::write(
        root.join("logseq/config.edn"),
        "{:tine/favorites-page \"favorites\"}\n",
    )
    .unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let graph = store.whole_graph().unwrap();
    assert_eq!(
        names(&graph.backlinks("Target").unwrap()),
        vec!["Notes".to_string()]
    );
    assert!(
        !names(&graph.unlinked_references("Target").unwrap()).contains(&"Favorites".to_string())
    );
    // The target page itself is still excluded from its own references.
    assert!(!names(&graph.backlinks("Notes").unwrap()).contains(&"Notes".to_string()));
    drop(graph);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
