use std::fs;
use tine_core::model::PageKind;
use tine_store::{EditKind, PageId, Resolved, SaveBase, SaveOutcome, Store};

#[cfg(unix)]
#[test]
fn path_saved_under_another_case_opens_disk_spelling() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("tine-case-path-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/contents.md"), "- body\n").unwrap();
    // A same-directory link simulates a case-insensitive volume's alias
    // lookup on Linux: canonicalize returns the file's actual disk spelling.
    symlink("contents.md", root.join("pages/Contents.md")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let read = store.page(&PageId::from("pages/Contents.md")).unwrap();
    assert_eq!(read.id.as_str(), "pages/contents.md");
    assert_eq!(read.doc.name, "contents");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn nonportable_graph_text_path_is_outside_page_scope() {
    let root = std::env::temp_dir().join(format!("tine-portable-scope-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("archive")).unwrap();
    fs::write(
        root.join("archive/Bad:Name.md"),
        "title:: Unsafe path\n\n- body\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Unsafe path", false),
        Resolved::Absent { .. }
    ));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn title_property_owns_page_name_through_store() {
    let root = std::env::temp_dir().join(format!("tine-title-identity-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(
        root.join("pages/Physical.md"),
        "title:: Effective\n\n- body\n",
    )
    .unwrap();
    fs::write(
        root.join("pages/Referrer.md"),
        "- [[Effective]] [[Renamed]] [[Saved]]\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Effective", false),
        Resolved::Existing { .. }
    ));
    let page = store
        .page_named("Effective", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(page.doc.name, "Effective");
    assert_eq!(page.id.as_str(), "pages/Physical.md");
    assert!(store
        .whole_graph()
        .unwrap()
        .backlinks("Effective")
        .unwrap()
        .iter()
        .any(|group| group.page == "Referrer"));
    fs::write(
        root.join("pages/Physical.md"),
        "title:: Renamed\n\n- body\n",
    )
    .unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Renamed", false),
        Resolved::Existing { .. }
    ));
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Effective", false),
        Resolved::Absent { .. }
    ));
    assert!(store
        .whole_graph()
        .unwrap()
        .backlinks("Renamed")
        .unwrap()
        .iter()
        .any(|group| group.page == "Referrer"));
    let mut renamed = store
        .page_named("Renamed", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(renamed.doc.name, "Renamed");
    renamed.doc.pre_block = Some("title:: Saved".into());
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &renamed.id,
            SaveBase::Existing(renamed.rev),
            &renamed.doc
        ),
        SaveOutcome::Saved(_)
    ));
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Saved", false),
        Resolved::Existing { .. }
    ));
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Renamed", false),
        Resolved::Absent { .. }
    ));
    assert!(store
        .whole_graph()
        .unwrap()
        .backlinks("Saved")
        .unwrap()
        .iter()
        .any(|group| group.page == "Referrer"));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ordinary_page_outside_standard_directories_is_discoverable() {
    let root = std::env::temp_dir().join(format!("tine-outside-pages-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("archive")).unwrap();
    fs::write(
        root.join("archive/Physical.md"),
        "title:: Archived\n\n- body\n",
    )
    .unwrap();
    for area in ["assets", "node_modules", "logseq/bak", "publish", ".hidden"] {
        fs::create_dir_all(root.join(area)).unwrap();
        fs::write(
            root.join(area).join("Excluded.md"),
            "title:: Excluded\n\n- body\n",
        )
        .unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Archived", false),
        Resolved::Existing { .. }
    ));
    let mut page = store
        .page_named("Archived", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(page.id.as_str(), "archive/Physical.md");
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Excluded", false),
        Resolved::Absent { .. }
    ));
    page.doc.blocks[0].raw = "edited".into();
    assert!(matches!(
        store.save(
            EditKind::SaveBlock,
            &page.id,
            SaveBase::Existing(page.rev),
            &page.doc
        ),
        SaveOutcome::Saved(_)
    ));
    assert_eq!(
        fs::read_to_string(root.join("archive/Physical.md")).unwrap(),
        "title:: Archived\n\n- edited\n"
    );
    fs::write(root.join("archive/Later.md"), "- another\n").unwrap();
    fs::write(root.join("archive/Markdown.markdown"), "- markdown\n").unwrap();
    fs::write(
        root.join("archive/Upper.ORG"),
        "#+TITLE: Org title\n* org\n",
    )
    .unwrap();
    fs::write(root.join("Top Level.md"), "- top\n").unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Later", false),
        Resolved::Existing { .. }
    ));
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Markdown", false),
        Resolved::Existing { .. }
    ));
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Org title", false),
        Resolved::Existing { .. }
    ));
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Top Level", false),
        Resolved::Existing { .. }
    ));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn filename_owner_wins_over_a_title_claimant() {
    let root = std::env::temp_dir().join(format!("tine-title-claimants-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/A.md"), "title:: Z\n\n- other\n").unwrap();
    fs::write(root.join("pages/Z.md"), "- canonical\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = match store.whole_graph().unwrap().resolve("Z", false) {
        Resolved::Existing { id, .. } => id,
        _ => panic!("Z must resolve"),
    };
    assert_eq!(id.as_str(), "pages/Z.md");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn safe_new_filename_and_legacy_existing_path_keep_master_bytes() {
    let root = std::env::temp_dir().join(format!("tine-safe-filenames-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/Template.md"), "- created\n").unwrap();
    fs::write(
        root.join("pages/Release 1.0.md"),
        "title:: Release 1.0\n\n- body\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let title = "2026-07-23_18:01:20";
    let id = match store.whole_graph().unwrap().resolve(title, false) {
        Resolved::Absent { id } => id,
        _ => panic!("new page expected"),
    };
    assert_eq!(id.as_str(), "pages/2026-07-23_18%3A01%3A20.md");
    let mut doc = store
        .page_named("Template", PageKind::Page)
        .unwrap()
        .unwrap()
        .doc;
    doc.name = title.into();
    doc.title = title.into();
    assert!(matches!(
        store.save(EditKind::ReplacePage, &id, SaveBase::CreateNew, &doc),
        SaveOutcome::Saved(_)
    ));
    assert_eq!(fs::read(root.join(id.as_str())).unwrap(), b"- created\n");
    drop(store);
    let store = Store::open(&root, Default::default()).unwrap().0;
    let mut reopened = store.page_named(title, PageKind::Page).unwrap().unwrap();
    assert_eq!(reopened.id, id);
    reopened.doc.blocks[0].raw = "edited and durable".into();
    assert!(matches!(
        store.save(
            EditKind::SaveBlock,
            &reopened.id,
            SaveBase::Existing(reopened.rev),
            &reopened.doc
        ),
        SaveOutcome::Saved(_)
    ));
    assert_eq!(
        fs::read(root.join(id.as_str())).unwrap(),
        b"- edited and durable\n"
    );
    drop(store);
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        store
            .page_named(title, PageKind::Page)
            .unwrap()
            .unwrap()
            .doc
            .blocks[0]
            .raw,
        "edited and durable"
    );
    let legacy = store
        .page_named("Release 1.0", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(legacy.id.as_str(), "pages/Release 1.0.md");
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &legacy.id,
            SaveBase::Existing(legacy.rev),
            &legacy.doc
        ),
        SaveOutcome::Saved(_) | SaveOutcome::Unchanged(_)
    ));
    assert_eq!(
        fs::read(root.join("pages/Release 1.0.md")).unwrap(),
        b"title:: Release 1.0\n\n- body\n"
    );
    assert!(!root.join("pages/Release 1%2E0.md").exists());
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// T1 (og-t): the page name comes from the preamble the page model reads,
/// which ends at the outline authority's first block. A leading unbulleted
/// heading is a block, so a `title::` under it is the heading's property
/// (OG `extract.cljc` `get-page-name`: title only from leading properties);
/// a bullet-looking line inside a fence is content, not the first block.
#[test]
fn page_name_agrees_with_the_outline_preamble() {
    let root = std::env::temp_dir().join(format!("tine-title-outline-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(
        root.join("pages/Physical.md"),
        "# Heading\ntitle:: Not The Page\n\n- body\n",
    )
    .unwrap();
    fs::write(
        root.join("pages/Fenced.md"),
        "```\n- not a block\n```\ntitle:: Fenced Title\n\n- body\n",
    )
    .unwrap();
    fs::write(
        root.join("pages/Referrer.md"),
        "- [[Physical]] [[Not The Page]] [[Fenced Title]]\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    assert!(matches!(
        graph.resolve("Physical", false),
        Resolved::Existing { .. }
    ));
    assert!(matches!(
        graph.resolve("Not The Page", false),
        Resolved::Absent { .. }
    ));
    assert!(matches!(
        graph.resolve("Fenced Title", false),
        Resolved::Existing { .. }
    ));
    let heading = store.page(&PageId::from("pages/Physical.md")).unwrap();
    assert_eq!(heading.doc.name, "Physical");
    assert_eq!(heading.doc.pre_block, None, "the heading is a block");
    let fenced = store.page(&PageId::from("pages/Fenced.md")).unwrap();
    assert_eq!(fenced.doc.name, "Fenced Title");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// T1 (og-t): a rename leaves a heading block's `title::` property alone; it
/// is not the page's title, so nothing binds it to the page name.
#[test]
fn rename_keeps_a_heading_blocks_title_property() {
    let root =
        std::env::temp_dir().join(format!("tine-title-heading-rename-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(
        root.join("pages/Physical.md"),
        "# Heading\ntitle:: Physical\n\n- body\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    tine_graph_features::pages::rename_page_expected(&store, None, "Physical", "Moved", None)
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("pages/Moved.md")).unwrap(),
        "# Heading\ntitle:: Physical\n\n- body\n"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
