//! Public entry-point regressions for parser-owned page metadata and literals.
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

#[test]
fn org_page_drawers_and_directives_contribute_backlinks_but_prose_does_not() {
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
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

/// D-20261018-IC1: `tine.icon::` is the page icon property; the OG-compatible `icon::` is the
/// fallback. A dotted key must parse in every page-property form, whichever is written first
/// wins by name (not by position), and a blank `tine.icon::` does not hide `icon::`.
#[test]
fn page_icons_prefer_tine_icon_and_fall_back_to_icon() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    for (name, raw) in [
        ("Tine.md", "tine.icon:: ![](../assets/a.png)\n\n- body\n"),
        ("Legacy.md", "icon:: 🏁\n\n- body\n"),
        ("Both.md", "icon:: 🏁\ntine.icon:: ⭐\n\n- body\n"),
        ("Blank.md", "tine.icon::\nicon:: 🏁\n\n- body\n"),
        ("Upper.md", "Tine.Icon:: ⭐\n\n- body\n"),
        (
            "Directive.org",
            "#+tine.icon: [[file:../assets/a.png]]\n\n* body\n",
        ),
        ("OrgBoth.org", "#+icon: 🏁\n#+tine.icon: ⭐\n\n* body\n"),
        (
            "Drawer.org",
            ":PROPERTIES:\n:icon: 🏁\n:tine.icon: ⭐\n:END:\n\n* body\n",
        ),
        ("Literal.md", "```\ntine.icon:: 👻\n```\n\n- body\n"),
        ("Owner.md", "tine.icon:: ⭐\nalias:: Nick\n\n- body\n"),
    ] {
        std::fs::write(dir.path().join("pages").join(name), raw).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let names = [
        "Tine",
        "Legacy",
        "Both",
        "Blank",
        "Upper",
        "Directive",
        "OrgBoth",
        "Drawer",
        "Literal",
        "Owner",
        "Nick",
    ];
    let icons = store
        .whole_graph()
        .unwrap()
        .page_icons(&names.map(String::from));
    let icon = |name: &str| icons.get(name).map(String::as_str);
    assert_eq!(icon("Tine"), Some("![](../assets/a.png)"));
    assert_eq!(icon("Legacy"), Some("🏁"), "icon:: is the fallback");
    assert_eq!(
        icon("Both"),
        Some("⭐"),
        "tine.icon:: wins wherever it is written"
    );
    assert_eq!(
        icon("Blank"),
        Some("🏁"),
        "a blank tine.icon:: does not hide icon::"
    );
    assert_eq!(icon("Upper"), Some("⭐"), "keys compare case-insensitively");
    assert_eq!(icon("Directive"), Some("[[file:../assets/a.png]]"));
    assert_eq!(icon("OrgBoth"), Some("⭐"));
    assert_eq!(icon("Drawer"), Some("⭐"));
    assert_eq!(icon("Literal"), None, "a fenced line is not a property");
    assert_eq!(
        icon("Nick"),
        Some("⭐"),
        "an alias inherits the owner's icon"
    );
}

#[test]
fn saving_org_conflict_marker_examples_preserves_the_literal_and_allows_the_edit() {
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
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
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
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
        &host,
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
