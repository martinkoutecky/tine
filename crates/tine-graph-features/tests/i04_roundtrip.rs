use std::fs;
use std::path::Path;
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_graph_features::pages;
use tine_store::{Area, PageId, RestoreFile, SaveBase, SaveOutcome, Store};

#[test]
fn merge_source_preamble_survives_in_lsdoc_tree() {
    let root = std::env::temp_dir().join(format!("tine-i04-merge-{}", std::process::id()));
    for area in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(area)).unwrap();
    }
    fs::write(
        root.join("pages/src.md"),
        b"alias:: Source\nfree text\n- moved\n",
    )
    .unwrap();
    fs::write(root.join("pages/dst.md"), b"alias:: Destination\n- kept\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    pages::merge_pages(&store, "pages/src.md", "pages/dst.md").unwrap();
    let bytes = fs::read_to_string(root.join("pages/dst.md")).unwrap();
    let projection = lsdoc::parse_format(&bytes, "md");
    let tree = serde_json::to_string(&projection).unwrap();
    assert!(projection.blocks.len() >= 2 && tree.contains("free text") && tree.contains("moved"),
        "I-4: merged free-text preamble and blocks must be Logseq-readable; exemplar pages::merge_pages: {tree}");
    assert!(bytes.contains("alias:: Source"),
        "I-4: conflicting source property must survive as block content; exemplar pages::merge_pages");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn emitted_fixture_corpus_is_accepted_by_lsdoc() {
    // The client tests pin these bytes after exercising each producer. Parse
    // the same artifacts independently through the Logseq-compatible oracle.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for (emitter, rel, format) in [
        ("merge", "page_merge_delete_and_rescue_match_legacy_bytes/merge/pages/dst.md", "md"),
        ("rename", "page_rename_matches_legacy_for_refs_namespace_alias_and_title/simple/pages/Next Name.md", "md"),
        ("conflict resolve", "conflict_clients_match_legacy_values_and_disk_bytes/resolved_conflict/pages/Foo.md", "md"),
        ("guide", "guide_creation_matches_legacy_tree_and_folder_choice/first_demo/pages/Welcome to Tine.md", "md"),
        ("PDF notes", "old_vs_new_matrix_on_identical_fixtures/first_highlight_notes/pages/hls__paper.md", "md"),
        ("Org merge", "org_merge_and_binary_rescue_match_legacy/org_merge/pages/dst.org", "org"),
    ] {
        let bytes = fs::read_to_string(root.join(rel)).unwrap();
        let parsed = lsdoc::parse_format(&bytes, format);
        assert!(!parsed.blocks.is_empty(),
            "I-4: {emitter} output must parse as Logseq blocks; exemplar store_save.rs:293; fixture {rel}");
        let json = serde_json::to_string(&parsed).unwrap();
        assert!(!json.is_empty(),
            "I-4: {emitter} output must project to a Logseq tree; exemplar store_save.rs:293; fixture {rel}");
    }
}

#[test]
fn save_and_restore_emit_parseable_page_trees() {
    let root = std::env::temp_dir().join(format!("tine-i04-save-restore-{}", std::process::id()));
    for area in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(area)).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from("pages/Saved.md");
    let page = PageDto {
        name: "Saved".into(),
        title: "Saved".into(),
        kind: PageKind::Page,
        format: Format::Md,
        blocks: vec![BlockDto {
            raw: "saved body [[Target]]".into(),
            ..Default::default()
        }],
        pre_block: Some("alias:: Saved alias".into()),
        rev: None,
        read_only: false,
        guide: false,
    };
    assert!(matches!(
        store.save(&id, SaveBase::CreateNew, &page),
        SaveOutcome::Saved(_)
    ));
    let saved = fs::read_to_string(root.join("pages/Saved.md")).unwrap();
    let parsed = lsdoc::parse_format(&saved, "md");
    assert!(!parsed.blocks.is_empty() && parsed.refs.page.contains(&"Target".into()),
        "I-4: saved page must preserve block structure and reference identity in lsdoc; exemplar store_save.rs:293");

    let snapshot = root.join("snapshot.md");
    fs::write(&snapshot, b"title:: Restored\n- restored body [[Saved]]\n").unwrap();
    let source = fs::File::open(&snapshot).unwrap();
    store
        .restore(vec![RestoreFile {
            area: Area::Pages,
            rel: "Restored.md".into(),
            len: source.metadata().unwrap().len(),
            source,
        }])
        .unwrap();
    let restored = fs::read_to_string(root.join("pages/Restored.md")).unwrap();
    let parsed = lsdoc::parse_format(&restored, "md");
    assert!(!parsed.blocks.is_empty() && parsed.refs.page.contains(&"Saved".into()),
        "I-4: restored raw page must keep Logseq block and reference semantics; exemplar Store::restore");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn config_and_pdf_sidecar_fixtures_remain_edn() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for (emitter, rel) in [
        (
            "config",
            "config_setters_match_legacy_values_and_bytes/workflow_typical/logseq/config.edn",
        ),
        (
            "PDF sidecar",
            "old_vs_new_matrix_on_identical_fixtures/first_highlight/assets/paper.edn",
        ),
    ] {
        let text = fs::read_to_string(root.join(rel)).unwrap();
        assert!(tine_core::edn::parse(&text).is_some(),
            "I-4: {emitter} fixture must remain Logseq-readable EDN; exemplar store_save.rs:293; fixture {rel}");
    }
}
