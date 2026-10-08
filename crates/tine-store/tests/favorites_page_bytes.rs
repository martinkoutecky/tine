//! Family 22 byte differential: the Favorites arrangement page, as the
//! frontend's `favoritesArrangementPage` sends it (same DTO shape as master's
//! `layoutPageDto`, with collapse carried in the raw as every og block does),
//! written through the real save path.
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::PageDto;
use tine_store::{EditKind, PageId, SaveBase, SavePagesOutcome, Store};

#[test]
fn favorites_arrangement_page_bytes() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-fav-bytes-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let block = |raw: &str, collapsed: bool, children: serde_json::Value| serde_json::json!({ "id": "", "raw": raw, "collapsed": collapsed, "children": children });
    let dto: PageDto = serde_json::from_value(serde_json::json!({
        "name": "Favorites", "kind": "page", "title": "Favorites",
        "pre_block": "tine/favorites:: true",
        "blocks": [
            block("[[Alpha]]", false, serde_json::json!([block("[[Beta]]", false, serde_json::json!([]))])),
            block("Work\ncollapsed:: true", true, serde_json::json!([block("[[Gamma]]", false, serde_json::json!([]))])),
        ],
    }))
    .unwrap();
    let id = PageId::from("pages/Favorites.md");
    let outcome = store.save_pages(&[(id, SaveBase::CreateNew, dto, vec![EditKind::CreatePage])]);
    assert!(
        matches!(outcome, SavePagesOutcome::Ok { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Favorites.md")).unwrap(),
        "tine/favorites:: true\n\n- [[Alpha]]\n\t- [[Beta]]\n- Work\n  collapsed:: true\n\t- [[Gamma]]\n"
    );
    store.close();
    // Read back: collapse returns as the flag, nesting as children.
    let store = Store::open(&root, Default::default()).unwrap().0;
    let read = store.page(&PageId::from("pages/Favorites.md")).unwrap().doc;
    assert_eq!(read.pre_block.as_deref(), Some("tine/favorites:: true"));
    assert!(read.blocks[1].collapsed && !read.blocks[0].collapsed);
    assert_eq!(read.blocks[0].children[0].raw, "[[Beta]]");
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn favorites_arrangement_page_in_an_org_preferred_graph() {
    // The DTO the frontend sends in an Org-preferred graph
    // (favoritesArrangementPage with format "org"): the store puts a new page
    // at pages/Favorites.org, so the marker is an Org file property and
    // collapse an Org drawer; both must read back as such.
    let root = std::env::temp_dir().join(format!("tine-fav-org-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:preferred-format \"Org\"}\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = match store.whole_graph().unwrap().resolve("Favorites", false) {
        tine_store::Resolved::Absent { id } => id,
        _ => panic!("Favorites is not absent"),
    };
    assert_eq!(id, "pages/Favorites.org");
    let dto: PageDto = serde_json::from_value(serde_json::json!({
        "name": "Favorites", "kind": "page", "title": "Favorites",
        "pre_block": "#+tine/favorites: true",
        "blocks": [
            { "id": "", "raw": "[[Alpha]]", "collapsed": false, "children": [
                { "id": "", "raw": "[[Beta]]", "collapsed": false, "children": [] }] },
            { "id": "", "raw": "Work\n:PROPERTIES:\n:collapsed: true\n:END:", "collapsed": true, "children": [
                { "id": "", "raw": "[[Gamma]]", "collapsed": false, "children": [] }] },
        ],
    }))
    .unwrap();
    let outcome = store.save_pages(&[(
        id.clone(),
        SaveBase::CreateNew,
        dto,
        vec![EditKind::CreatePage],
    )]);
    assert!(
        matches!(outcome, tine_store::SavePagesOutcome::Ok { .. }),
        "{outcome:?}"
    );
    let files: Vec<_> = fs::read_dir(root.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(files, vec![std::ffi::OsString::from("Favorites.org")]);
    assert_eq!(
        fs::read_to_string(root.join("pages/Favorites.org")).unwrap(),
        "#+tine/favorites: true\n* [[Alpha]]\n** [[Beta]]\n* Work\n:PROPERTIES:\n:collapsed: true\n:END:\n** [[Gamma]]\n"
    );
    store.close();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let read = store.page(&id).unwrap().doc;
    assert_eq!(read.pre_block.as_deref(), Some("#+tine/favorites: true"));
    let blocks: Vec<_> = read
        .blocks
        .iter()
        .map(|b| (b.raw.as_str(), b.collapsed, b.children.len()))
        .collect();
    assert_eq!(
        blocks,
        vec![
            ("[[Alpha]]", false, 1),
            ("Work\n:PROPERTIES:\n:collapsed: true\n:END:", true, 1)
        ]
    );
    store.close();
    let _ = fs::remove_dir_all(&root);
}
