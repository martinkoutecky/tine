//! Public entry-point regressions for parser-owned page metadata and literals.
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

#[test]
fn org_page_drawers_and_directives_contribute_backlinks_but_prose_does_not() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    for (name, raw) in [
        ("Outside", ":tags: [[Ghost]]\n\n* body\n"),
        (
            "Drawer",
            ":PROPERTIES:\n:tags: [[Ghost]]\n:END:\n\n* body\n",
        ),
        ("Directive", "#+TAGS: [[Ghost]]\n\n* body\n"),
        (
            "DrawerColon",
            ":PROPERTIES:\n:tags: [[Ghost]], scheme::value\n:END:\n\n* body\n",
        ),
        (
            "DirectiveColon",
            "#+TAGS: [[Ghost]], scheme::value\n\n* body\n",
        ),
        (
            "Literal",
            "#+BEGIN_SRC\n#+TAGS: [[Ghost]]\n#+END_SRC\n\n* body\n",
        ),
    ] {
        std::fs::write(dir.path().join(format!("pages/{name}.org")), raw).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    let backlinks = graph.backlinks("Ghost").unwrap();
    let names: Vec<_> = backlinks.iter().map(|g| g.page.as_str()).collect();
    assert!(
        names.contains(&"Drawer"),
        "I-12: valid Org drawer lost its backlink: {names:?}"
    );
    assert!(
        names.contains(&"Directive"),
        "Org directives contribute page properties"
    );
    for name in ["DrawerColon", "DirectiveColon"] {
        assert!(
            names.contains(&name),
            "I-12: property values containing :: retain parser ownership: {names:?}"
        );
    }
    assert!(
        !names.contains(&"Outside"),
        "Org colon prose is not a property"
    );
    assert!(
        !names.contains(&"Literal"),
        "literal directives are not properties"
    );
}

#[test]
fn page_icons_use_the_file_format_and_parser_property_ownership() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    for (name, raw) in [
        ("Markdown.md", "#+ICON: ⭐\n\n- body\n"),
        ("Org.org", "#+ICON: ⭐\n\n* body\n"),
        ("Drawer.org", ":PROPERTIES:\n:icon: 🏁\n:END:\n\n* body\n"),
        ("Literal.md", "```\nicon:: 👻\n```\n\n- body\n"),
    ] {
        std::fs::write(dir.path().join("pages").join(name), raw).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let icons = store.whole_graph().unwrap().page_icons(&[
        "Markdown".into(),
        "Org".into(),
        "Drawer".into(),
        "Literal".into(),
    ]);
    assert!(
        !icons.contains_key("Markdown"),
        "Markdown #+ICON is prose: {icons:?}"
    );
    assert_eq!(icons.get("Org").map(String::as_str), Some("⭐"));
    assert_eq!(icons.get("Drawer").map(String::as_str), Some("🏁"));
    assert!(!icons.contains_key("Literal"));
}

#[test]
fn saving_org_conflict_marker_examples_preserves_the_literal_and_allows_the_edit() {
    for newline in ["\n", "\r\n"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        let literal =
            "#+BEGIN_SRC text\n<<<<<<< HEAD\nexample\n=======\nother\n>>>>>>> topic\n#+END_SRC\n\n"
                .replace("\n", newline);
        let file = dir.path().join("pages/Examples.org");
        std::fs::write(&file, format!("{literal}* body{newline}")).unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let id = PageId::from("pages/Examples.org");
        assert!(tine_graph_features::conflicts::conflict_inventory(&store)
            .unwrap()
            .vcs_markers
            .is_empty());
        assert!(
            tine_graph_features::conflicts::vcs_marker_conflict_diff(&store, id.as_str())
                .unwrap()
                .is_none()
        );
        let read = store.page(&id).unwrap();
        let mut edited = read.doc;
        edited.blocks[0].raw = "edited".into();
        let result = store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &edited,
        );
        assert!(
            matches!(result, SaveOutcome::Saved { .. }),
            "Org literal examples must remain editable: {result:?}"
        );
        let after = std::fs::read_to_string(&file).unwrap();
        assert!(
            after.starts_with(&literal),
            "literal bytes changed: {after:?}"
        );
        assert_eq!(
            after,
            format!("{literal}* edited\n"),
            "I-4: a save changes only the edited block, preserving LF/CRLF example bytes"
        );
    }
}

#[test]
fn plain_references_never_match_an_unaccented_prefix_of_an_nfd_grapheme() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("pages/Source.md"),
        "- Cafe\u{301} and Café\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    assert!(
        graph.unlinked_references("Cafe").unwrap().is_empty(),
        "I-4: a combining accent belongs to the matched grapheme"
    );
    let matching = graph.unlinked_references("Café").unwrap();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].page, "Source");
}

#[test]
fn org_real_conflicts_beside_literal_examples_are_inventory_and_resolution_candidates() {
    use std::collections::HashMap;
    use tine_graph_features::conflicts;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    let literal = "#+BEGIN_EXAMPLE\n<<<<<<< HEAD\n=======\n>>>>>>> demo\n#+END_EXAMPLE\n\n";
    let file = dir.path().join("pages/Real.org");
    std::fs::write(
        &file,
        format!("{literal}<<<<<<< HEAD\n* mine\n=======\n* theirs\n>>>>>>> topic\n"),
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let inventory = conflicts::conflict_inventory(&store).unwrap();
    assert_eq!(inventory.vcs_markers.len(), 1);
    let reviewed = conflicts::vcs_marker_conflict_diff(&store, "pages/Real.org")
        .unwrap()
        .unwrap();
    let decisions: HashMap<_, _> = reviewed
        .diff
        .rows
        .iter()
        .map(|row| (row.id.clone(), "both".to_string()))
        .collect();
    conflicts::resolve_vcs_marker_conflict(
        &store,
        "pages/Real.org",
        &decisions,
        &reviewed.diff.base_rev,
        "union",
    )
    .unwrap();
    let after = std::fs::read_to_string(&file).unwrap();
    assert!(
        after.starts_with(literal),
        "resolution must retain the example: {after:?}"
    );
    assert!(after.contains("* mine") && after.contains("* theirs"));
    assert!(conflicts::conflict_inventory(&store)
        .unwrap()
        .vcs_markers
        .is_empty());
}
