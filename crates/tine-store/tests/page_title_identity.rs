use std::fs;
use tine_core::model::PageKind;
use tine_store::{EditKind, PageId, Resolved, SaveBase, Store};

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

#[cfg(unix)]
#[test]
fn path_saved_under_another_case_opens_disk_spelling() {
    let _integration_guard = crate::integration_lock();
    use std::os::unix::fs::symlink;

    let root = crate::test_support::fresh_dir("tine-case-path-");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/contents.md"), "- body\n").unwrap();
    // Use the real alias on a folding volume. A same-directory link models
    // that lookup on a sensitive volume such as Linux's usual temp volume.
    if !root.join("pages/Contents.md").exists() {
        symlink("contents.md", root.join("pages/Contents.md")).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    let read = store.page(&PageId::from("pages/Contents.md")).unwrap();
    assert_eq!(read.id.as_str(), "pages/contents.md");
    // OG names pages/contents.* "Contents" (REG-CM-CONTENTS-PAGE-NAME).
    assert_eq!(read.doc.name, "Contents");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

// Linux/Android model the resolver with a link when needed; macOS/iOS use
// their native alias when available. Windows normalization moves are witnessed
// by og_gh609_case_rename and transaction::move_file::spelling_tests.
#[cfg(all(
    not(target_os = "windows"),
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )
))]
#[test]
fn normalization_alias_opens_only_the_listed_disk_spelling() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    let listed = "pages/cafe\u{301}.md";
    let alias = "pages/CAFÉ.md";
    fs::write(dir.path().join(listed), "title:: café\n- body\n").unwrap();
    if !dir.path().join(alias).exists() {
        std::os::unix::fs::symlink("cafe\u{301}.md", dir.path().join(alias)).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let read = store.page(&PageId::from(alias)).unwrap();
    assert_eq!(read.id.as_str(), listed);
    let Resolved::Existing { id, others } = store.whole_graph().unwrap().resolve("café", false)
    else {
        panic!("the listed page must resolve");
    };
    assert_eq!(id.as_str(), listed);
    assert!(others.is_empty(), "a resolver alias must not become a twin");
    store.close();
}

#[cfg(unix)]
#[test]
fn nonportable_graph_text_path_is_outside_page_scope() {
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-portable-scope-");
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
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-title-identity-");
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
    let store = std::sync::Arc::new(store);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
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
    host_writer
        .save(
            EditKind::ReplacePage,
            &renamed.id,
            SaveBase::Existing(renamed.rev),
            &renamed.doc,
        )
        .unwrap();
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
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-outside-pages-");
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
    let store = std::sync::Arc::new(store);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
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
    host_writer
        .save(
            EditKind::SaveBlock,
            &page.id,
            SaveBase::Existing(page.rev),
            &page.doc,
        )
        .unwrap();
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
fn the_first_path_wins_even_when_its_claim_comes_from_a_title() {
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-title-claimants-");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/A.md"), "title:: Z\n\n- other\n").unwrap();
    fs::write(root.join("pages/Z.md"), "- canonical\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = match store.whole_graph().unwrap().resolve("Z", false) {
        Resolved::Existing { id, .. } => id,
        _ => panic!("Z must resolve"),
    };
    assert_eq!(id.as_str(), "pages/A.md");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn safe_new_filename_and_legacy_existing_path_keep_master_bytes() {
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-safe-filenames-");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("pages/Template.md"), "- created\n").unwrap();
    fs::write(
        root.join("pages/Release 1.0.md"),
        "title:: Release 1.0\n\n- body\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let store = std::sync::Arc::new(store);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
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
    host_writer
        .save(EditKind::ReplacePage, &id, SaveBase::CreateNew, &doc)
        .unwrap();
    assert_eq!(fs::read(root.join(id.as_str())).unwrap(), b"- created\n");
    drop(store);
    let store = Store::open(&root, Default::default()).unwrap().0;
    let store = std::sync::Arc::new(store);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
    let mut reopened = store.page_named(title, PageKind::Page).unwrap().unwrap();
    assert_eq!(reopened.id, id);
    reopened.doc.blocks[0].raw = "edited and durable".into();
    host_writer
        .save(
            EditKind::SaveBlock,
            &reopened.id,
            SaveBase::Existing(reopened.rev),
            &reopened.doc,
        )
        .unwrap();
    assert_eq!(
        fs::read(root.join(id.as_str())).unwrap(),
        b"- edited and durable\n"
    );
    drop(store);
    let store = Store::open(&root, Default::default()).unwrap().0;
    let store = std::sync::Arc::new(store);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
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
        host_writer.save(
            EditKind::ReplacePage,
            &legacy.id,
            SaveBase::Existing(legacy.rev),
            &legacy.doc
        ),
        Ok(_)
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
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-title-outline-");
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
    let _integration_guard = crate::integration_lock();
    let root = crate::test_support::fresh_dir("tine-title-heading-rename-");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(
        root.join("pages/Physical.md"),
        "# Heading\ntitle:: Physical\n\n- body\n",
    )
    .unwrap();
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    hosted(&store, |host| {
        tine_graph_features::pages::rename_page_expected(&store, host, "Physical", "Moved", None)
    })
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("pages/Moved.md")).unwrap(),
        "# Heading\ntitle:: Physical\n\n- body\n"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// CM-NAMES (1), D-20261103-CMA: a file name with a malformed percent escape
/// or (triple-lowbar) an empty namespace part gets OG's page name
/// (graph_parser/util.cljs `tri-lb-title-parsing`, `legacy-title-parsing` at
/// OG 6e7afa8eb); valid UTF-8 escapes stay decoded ("café", a deliberate
/// difference from OG's triple-lowbar codec). A page Tine creates under such a
/// name round-trips: the file it proposes reads back as the same page.
#[test]
fn file_names_with_malformed_escapes_and_empty_namespace_parts_get_ogs_name() {
    let _integration_guard = crate::integration_lock();
    let cases: [(&str, &str, &str); 8] = [
        // (file stem, triple-lowbar name, Legacy name)
        ("%E9", "%E9", "%E9"),
        ("bad%E9", "bad%E9", "bad%E9"),
        ("___lead", "lead", "___lead"),
        ("a___", "a", "a___"),
        ("a______b", "a/b", "a______b"),
        ("caf%C3%A9", "café", "café"),
        ("50%25 of 100%", "50% of 100%", "50%25 of 100%"),
        ("x%2F%2Fy", "x/y", "x//y"),
    ];
    for (config, triple) in [
        (Some("{:file/name-format :triple-lowbar}\n"), true),
        (Some("{:file/name-format :legacy}\n"), false),
        (None, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pages")).unwrap();
        if let Some(config) = config {
            fs::create_dir_all(dir.path().join("logseq")).unwrap();
            fs::write(dir.path().join("logseq/config.edn"), config).unwrap();
        }
        for (stem, ..) in cases {
            fs::write(dir.path().join(format!("pages/{stem}.md")), "- body\n").unwrap();
        }
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let graph = store.whole_graph().unwrap();
        for (stem, tri_name, legacy_name) in cases {
            let want = if triple { tri_name } else { legacy_name };
            let id = PageId::from(format!("pages/{stem}.md"));
            let read = store.page(&id).unwrap();
            assert_eq!(read.doc.name, want, "{stem}.md ({config:?})");
            match graph.resolve(want, false) {
                Resolved::Existing { id: found, .. } => {
                    assert_eq!(found, id, "{want:?} ({config:?})")
                }
                _ => panic!("{want:?} does not resolve to {stem}.md ({config:?})"),
            }
        }
        // Creating such a page: the proposed file reads back as that page.
        for name in ["%E9", "bad%E9", "lead", "50%25 of 100%"] {
            let Resolved::Absent { id } = graph.resolve(&format!("{name} new"), false) else {
                panic!("{name} new exists");
            };
            let stem = id
                .as_str()
                .strip_prefix("pages/")
                .and_then(|s| s.strip_suffix(".md"))
                .unwrap()
                .to_owned();
            let format = if triple {
                tine_core::config::FileNameFormat::TripleLowbar
            } else {
                tine_core::config::FileNameFormat::Legacy
            };
            assert_eq!(
                tine_core::model::decode_page_name(&stem, format),
                format!("{name} new"),
                "{stem} ({config:?})"
            );
        }
    }
}
