//! Integration tests against the on-disk demo graph (standard layout).

use std::path::PathBuf;
use tine_core::{ActivationIntent, Graph, PageDto};

/// Make `dto` an EDITOR's DTO, the way the frontend does.
///
/// Since GH #254 increment 3 a loaded page and a live editor are different
/// things: reading alone mints no identity, so a read for export, preview or
/// hydration cannot inherit an editor's override authority. A test that
/// force-saves is modelling a user answering a conflict banner, so it has to
/// activate like one. Works for an absent page too — activation resolves a
/// prospective target and writes nothing.
fn as_editor(graph: &Graph, dto: &mut PageDto) {
    let handle = graph
        .activate_editor(&dto.path, ActivationIntent::Replace, dto.rev.as_deref())
        .expect("the target is inside the graph");
    dto.activation = Some(handle.activation.as_u64());
}

fn demo_graph() -> Graph {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo-graph");
    Graph::open(root)
}

#[test]
fn lists_journals_and_pages() {
    let g = demo_graph();
    let pages = g.list_pages();
    assert!(pages.iter().any(|p| p.name == "logseq-claude"));
    let journals = g.journals_desc();
    // Newest first.
    assert_eq!(
        journals.first().map(|j| j.name.as_str()),
        Some("Jun 14th, 2026")
    );
    assert!(journals.len() >= 2);
}

#[test]
fn gh246_discovers_and_loads_nonempty_document_outside_configured_roots() {
    let root = std::env::temp_dir().join(format!("tine-gh246-graph-wide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("archive/client")).unwrap();
    std::fs::write(
        root.join("archive/client/Plan.markdown"),
        "- irreplaceable external bytes\n",
    )
    .unwrap();

    let graph = Graph::open(&root);
    let pages = graph.list_pages();
    let entry = pages
        .iter()
        .cloned()
        .into_iter()
        .find(|entry| entry.rel_path == "archive/client/Plan.markdown")
        .unwrap_or_else(|| {
            panic!(
                "eligible external graph document must be discoverable; pages={pages:?}, failures={:?}",
                graph.page_index_failures()
            )
        });
    assert_eq!(entry.name, "Plan");
    let page = graph
        .load_by_path("archive/client/Plan.markdown")
        .unwrap()
        .expect("eligible external graph document must load by its exact path");
    assert_eq!(page.path, "archive/client/Plan.markdown");
    assert_eq!(page.blocks[0].raw, "irreplaceable external bytes");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn graph_text_scope_discovers_all_formats_titles_dates_and_applies_exclusions() {
    let root = std::env::temp_dir().join(format!(
        "tine-graph-text-scope-layout-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for directory in [
        "content/pages/nested",
        "content/journals",
        "external/multi/level",
        "archive/private",
        "node_modules/pkg",
        "deep/node_modules/pkg",
        "logseq/.recycle",
        "logseq/bak",
        "logseq/version-files",
        "assets",
        "publish",
        ".tine-sync",
        "logseq/.tine-trash/pages",
        ".dot",
    ] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    std::fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"content/pages\"\n\
          :journals-directory \"content/journals\"\n\
          :hidden [\"archive/private\" \"scratch\"]}\n",
    )
    .unwrap();
    let accepted = [
        ("Root.md", "- root\n"),
        ("UPPER.MD", "- uppercase root\n"),
        ("external/multi/level/Long.markdown", "- markdown\n"),
        (
            "external/multi/level/Mixed.Markdown",
            "- mixed-case markdown\n",
        ),
        ("external/multi/level/Outline.org", "* org\n"),
        ("content/pages/nested/Upper.ORG", "* uppercase org\n"),
        ("external/Foo.Bar.md", "- legacy dotted namespace\n"),
        ("content/pages/nested/Configured.md", "- configured\n"),
        ("logseq/allowed.md", "- allowed\n"),
        (
            "external/Title Source.md",
            "title:: Parsed Title\n\n- titled\n",
        ),
        ("external/2026_07_25.md", "- semantic journal\n"),
    ];
    for (path, bytes) in accepted {
        if let Some(parent) = root.join(path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(root.join(path), bytes).unwrap();
    }
    for path in [
        "archive/private/hidden.md",
        "scratch-hidden.md",
        "node_modules/pkg/hidden.md",
        "deep/node_modules/pkg/hidden.org",
        "logseq/.recycle/hidden.md",
        "logseq/bak/hidden.md",
        "logseq/version-files/hidden.md",
        "assets/hidden.md",
        "publish/hidden.org",
        ".tine-sync/hidden.md",
        "logseq/.tine-trash/pages/hidden.md",
        ".dot/hidden.md",
        ".hidden.md",
        "external/Page.sync-conflict-20260725-120000-ABCDEF.md",
        "external/Page (conflicted copy 2026-07-25).md",
    ] {
        if let Some(parent) = root.join(path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(root.join(path), "- excluded\n").unwrap();
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("Root.md"), root.join("external/symlink.md")).unwrap();
    }

    let graph = Graph::open(&root);
    assert_eq!(graph.graph_text_scope_version(), 1);
    let pages = graph.list_pages();
    let paths = pages
        .iter()
        .map(|entry| entry.rel_path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for (path, _) in accepted {
        assert!(
            paths.contains(path),
            "{path} missing from {paths:?}; failures={:?}",
            graph.page_index_failures()
        );
    }
    assert_eq!(
        pages
            .iter()
            .find(|entry| entry.rel_path == "external/Title Source.md")
            .map(|entry| entry.name.as_str()),
        Some("Parsed Title")
    );
    assert_eq!(
        pages
            .iter()
            .find(|entry| entry.rel_path == "external/Foo.Bar.md")
            .map(|entry| entry.name.as_str()),
        Some("Foo/Bar")
    );
    let journal = pages
        .iter()
        .find(|entry| entry.rel_path == "external/2026_07_25.md")
        .unwrap();
    assert_eq!(journal.kind, tine_core::PageKind::Journal);
    assert!(journal.date_key.is_some());
    let upper_org = graph
        .load_by_path("content/pages/nested/Upper.ORG")
        .unwrap()
        .unwrap();
    assert_eq!(upper_org.blocks[0].raw, "uppercase org");
    assert_eq!(pages.len(), accepted.len(), "{pages:?}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn case_insensitive_extensions_edit_exact_paths_without_lowercase_duplicates() {
    let root = std::env::temp_dir().join(format!(
        "tine-graph-text-case-extension-save-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for directory in ["pages", "journals", "external/deep"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    for (relative, original, edited, serialized_prefix, lowercase_duplicate) in [
        ("Root.MD", "- root\n", "root edited", "- ", "Root.md"),
        (
            "external/deep/Mixed.Markdown",
            "- nested\n",
            "nested edited",
            "- ",
            "external/deep/Mixed.markdown",
        ),
        (
            "pages/Outline.ORG",
            "* outline\n",
            "outline edited",
            "* ",
            "pages/Outline.org",
        ),
    ] {
        let path = root.join(relative);
        std::fs::write(&path, original).unwrap();
        let graph = Graph::open(&root);
        let mut page = graph.load_by_path(relative).unwrap().unwrap();
        page.blocks[0].raw = edited.into();
        graph.save_page(&page, page.rev.as_deref()).unwrap();
        assert!(path.is_file());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .starts_with(&format!("{serialized_prefix}{edited}")));
        let lowercase_duplicate = root.join(lowercase_duplicate);
        assert!(!std::fs::read_dir(lowercase_duplicate.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() == lowercase_duplicate.file_name().unwrap()));
    }
    assert!(!root.join("pages/Root.md").exists());
    assert!(!root.join("pages/Mixed.markdown").exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn external_graph_text_save_keeps_exact_path_extension_and_rejects_stale_bytes() {
    let root = std::env::temp_dir().join(format!("tine-graph-text-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("external/deep")).unwrap();
    let path = root.join("external/deep/Exact.markdown");
    std::fs::write(&path, "- original\n").unwrap();

    let graph = Graph::open(&root);
    let mut page = graph
        .load_by_path("external/deep/Exact.markdown")
        .unwrap()
        .unwrap();
    page.blocks[0].raw = "saved in place".into();
    let revision = graph.save_page(&page, page.rev.as_deref()).unwrap();
    assert_eq!(page.path, "external/deep/Exact.markdown");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "- saved in place\n"
    );
    assert!(!root.join("pages/Exact.md").exists());

    page.rev = Some(revision);
    page.blocks[0].raw = "stale overwrite attempt".into();
    std::fs::write(&path, "- external winner\n").unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        graph
            .save_page(&page, page.rev.as_deref())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);

    #[cfg(unix)]
    {
        let mut identity_bound = graph
            .load_by_path("external/deep/Exact.markdown")
            .unwrap()
            .unwrap();
        identity_bound.blocks[0].raw = "keep mine over a republished inode".into();
        as_editor(&graph, &mut identity_bound);
        std::fs::write(&path, "- shown conflict\n").unwrap();
        graph
            .save_page(&identity_bound, identity_bound.rev.as_deref())
            .unwrap_err();
        let shown = graph
            .outstanding_conflict_override(&identity_bound)
            .unwrap()
            .expect("the refused save names the conflict shown to this editor");

        // A syncer republishes the SAME bytes by temp+rename: new inode, state
        // the user was shown unchanged. "Keep mine" must go through. Refusing
        // would be stricter than an ordinary save, which treats a same-byte
        // republication as the state it already has (GH #254 increment 2).
        let replacement = root.join("external/deep/.replacement.markdown");
        std::fs::write(&replacement, "- shown conflict\n").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        graph
            .force_save_page_at_revision(&identity_bound, identity_bound.rev.as_deref(), shown)
            .unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("keep mine over a republished inode"));

        // A DIFFERENT-byte winner on a new inode is still refused, and the file
        // is left exactly as that winner wrote it.
        let mut second = graph
            .load_by_path("external/deep/Exact.markdown")
            .unwrap()
            .unwrap();
        second.blocks[0].raw = "must not replace a different winner".into();
        // A live editor too, so the refusal below is the byte-binding refusal
        // this test is about and not merely a missing activation.
        as_editor(&graph, &mut second);
        std::fs::write(&path, "- second shown conflict\n").unwrap();
        graph.save_page(&second, second.rev.as_deref()).unwrap_err();
        let shown = graph
            .outstanding_conflict_override(&second)
            .unwrap()
            .expect("the refused save names the conflict shown to this editor");
        let foreign = root.join("external/deep/.foreign.markdown");
        std::fs::write(&foreign, "- different winner\n").unwrap();
        std::fs::rename(&foreign, &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert_eq!(
            graph
                .force_save_page_at_revision(&second, second.rev.as_deref(), shown)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn invalid_utf8_never_loads_as_a_blank_writable_page() {
    let root = std::env::temp_dir().join(format!("tine-graph-text-utf8-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("external")).unwrap();
    let path = root.join("external/Invalid.md");
    std::fs::write(&path, [0xff, 0xfe, b'\n']).unwrap();

    let graph = Graph::open(&root);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        graph
            .load_by_path("external/Invalid.md")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn duplicate_effective_identity_keeps_exact_owners_readable_and_external_mutations_explicit() {
    let root =
        std::env::temp_dir().join(format!("tine-graph-text-conflict-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("external/a")).unwrap();
    std::fs::create_dir_all(root.join("external/b")).unwrap();
    std::fs::write(root.join("external/a/Twin.md"), "- A\n").unwrap();
    std::fs::write(root.join("external/b/Other.org"), "#+TITLE: Twin\n* B\n").unwrap();

    let graph = Graph::open(&root);
    let mut first = graph
        .load_by_path("external/a/Twin.md")
        .unwrap()
        .expect("first physical owner remains readable");
    let second = graph
        .load_by_path("external/b/Other.org")
        .unwrap()
        .expect("second physical owner remains readable");
    assert_eq!(first.blocks[0].raw, "A");
    assert_eq!(second.blocks[0].raw, "B");
    assert_eq!(
        graph
            .find_entry("Twin", tine_core::PageKind::Page)
            .expect("logical lookup retains a deterministic owner")
            .rel_path,
        "external/a/Twin.md"
    );
    first.blocks[0].raw = "A edited through its retained owner".into();
    graph
        .save_page(&first, first.rev.as_deref())
        .expect("an exact retained owner remains writable despite a semantic duplicate");
    assert_eq!(
        std::fs::read_to_string(root.join("external/a/Twin.md")).unwrap(),
        "- A edited through its retained owner\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("external/b/Other.org")).unwrap(),
        "#+TITLE: Twin\n* B\n"
    );
    let mut unpinned = first.clone();
    unpinned.path.clear();
    unpinned.blocks[0].raw = "must not choose an owner".into();
    assert_eq!(
        graph
            .save_page(&unpinned, unpinned.rev.as_deref())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert!(!root.join("pages/Twin.md").exists());
    let before = std::fs::read(root.join("external/a/Twin.md")).unwrap();
    assert_eq!(
        graph
            .rename_page_expected("Twin", "Renamed", Some("external/a/Twin.md"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Unsupported
    );
    assert_eq!(
        graph
            .delete_page_expected(
                "Twin",
                tine_core::PageKind::Page,
                Some("external/a/Twin.md")
            )
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Unsupported
    );
    assert_eq!(
        std::fs::read(root.join("external/a/Twin.md")).unwrap(),
        before
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn basename_and_same_stem_extension_conflicts_remain_exactly_readable() {
    for (tag, files) in [
        (
            "basename",
            vec![
                ("external/a/Dup.md", "- A\n"),
                ("external/b/Dup.md", "- B\n"),
            ],
        ),
        (
            "extension",
            vec![
                ("external/Dup.md", "- md\n"),
                ("external/Dup.org", "* org\n"),
            ],
        ),
    ] {
        let root = std::env::temp_dir().join(format!(
            "tine-graph-text-{tag}-conflict-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        for (path, content) in files {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), content).unwrap();
        }
        let graph = Graph::open(&root);
        let pages = graph.list_pages();
        assert_eq!(pages.len(), 2);
        for entry in &pages {
            let loaded = graph
                .load_by_path(&entry.rel_path)
                .unwrap()
                .expect("each colliding physical owner remains readable");
            assert_eq!(loaded.path, entry.rel_path);
        }
        assert_eq!(
            graph
                .find_entry("Dup", tine_core::PageKind::Page)
                .expect("logical lookup keeps its stable first owner")
                .rel_path,
            pages[0].rel_path
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

#[cfg(unix)]
#[test]
fn portable_case_nfc_and_file_identity_aliases_are_readable_but_non_writable() {
    for (tag, first, second) in [
        ("case", "External/Page.md", "external/page.md"),
        (
            "case-extension",
            "external/Extension.MD",
            "external/extension.md",
        ),
        ("nfc", "external/Caf\u{e9}.md", "external/Cafe\u{301}.md"),
        (
            "ancestor-nfc",
            "Ext\u{e9}rnal/Page.md",
            "Exte\u{301}rnal/page.md",
        ),
        (
            "german-sharp-s",
            "external/Straße.md",
            "external/STRASSE.md",
        ),
        ("greek-sigma", "external/Σ.md", "external/ς.md"),
    ] {
        let root = std::env::temp_dir().join(format!(
            "tine-graph-text-portable-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::create_dir_all(root.join(first).parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join(second).parent().unwrap()).unwrap();
        std::fs::write(root.join(first), "- first\n").unwrap();
        std::fs::write(root.join(second), "- second\n").unwrap();
        let graph = Graph::open(&root);
        assert_eq!(graph.list_pages().len(), 2);
        for (path, expected) in [(first, "first"), (second, "second")] {
            let mut page = graph
                .load_by_path(path)
                .unwrap()
                .expect("portable aliases stay readable for recovery");
            assert_eq!(page.blocks[0].raw, expected);
            page.blocks[0].raw = "must not publish".into();
            assert_eq!(
                graph
                    .save_page(&page, page.rev.as_deref())
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::AlreadyExists
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.join(first)).unwrap(),
            "- first\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(second)).unwrap(),
            "- second\n"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    let root =
        std::env::temp_dir().join(format!("tine-graph-text-hardlink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("external")).unwrap();
    std::fs::write(root.join("external/One.md"), "- shared inode\n").unwrap();
    std::fs::hard_link(root.join("external/One.md"), root.join("external/Two.md")).unwrap();
    let graph = Graph::open(&root);
    assert_eq!(graph.list_pages().len(), 2);
    for path in ["external/One.md", "external/Two.md"] {
        let mut page = graph
            .load_by_path(path)
            .unwrap()
            .expect("same-inode aliases stay readable for recovery");
        assert_eq!(page.blocks[0].raw, "shared inode");
        page.blocks[0].raw = "must not publish".into();
        assert_eq!(
            graph
                .save_page(&page, page.rev.as_deref())
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::AlreadyExists
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join("external/One.md")).unwrap(),
        "- shared inode\n"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn cold_filename_identity_and_warm_watcher_title_kind_changes_stay_coherent() {
    let root = std::env::temp_dir().join(format!(
        "tine-graph-text-effective-identity-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("external")).unwrap();
    std::fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"content/pages\"\n\
          :journals-directory \"content/journals\"\n\
          :file/name-format :legacy}\n",
    )
    .unwrap();
    std::fs::write(root.join("external/Foo.Bar.md"), "- namespace\n").unwrap();
    let mutable = root.join("external/Mutable.md");
    std::fs::write(&mutable, "title:: Initial Title\n\n- before\n").unwrap();

    let graph = Graph::open(&root);
    let cold = graph.list_pages();
    assert_eq!(
        cold.iter()
            .find(|entry| entry.rel_path == "external/Foo.Bar.md")
            .map(|entry| entry.name.as_str()),
        Some("Foo/Bar")
    );
    graph.warm_cache();

    std::fs::write(&mutable, "title:: 2026_07_25\n\n- dated\n").unwrap();
    let dated = graph
        .sync_file_checked(&mutable)
        .unwrap()
        .expect("watcher refresh must report the changed effective identity");
    assert_eq!(dated.kind, tine_core::PageKind::Journal);
    assert_eq!(dated.name, "Jul 25th, 2026");
    assert!(dated.date_key.is_some());
    graph.with_pages(|pages| {
        let (entry, document) = pages
            .iter()
            .find(|(entry, _)| entry.rel_path == "external/Mutable.md")
            .unwrap();
        assert_eq!(entry.kind, tine_core::PageKind::Journal);
        assert_eq!(entry.name, "Jul 25th, 2026");
        assert!(entry.date_key.is_some());
        assert_eq!(document.roots[0].raw, "dated");
    });

    std::fs::write(&mutable, "title:: Renamed Outside\n\n- after\n").unwrap();
    let renamed = graph.sync_file_checked(&mutable).unwrap().unwrap();
    assert_eq!(renamed.kind, tine_core::PageKind::Page);
    assert_eq!(renamed.name, "Renamed Outside");
    assert_eq!(renamed.date_key, None);
    graph.with_pages(|pages| {
        let (entry, document) = pages
            .iter()
            .find(|(entry, _)| entry.rel_path == "external/Mutable.md")
            .unwrap();
        assert_eq!(entry.kind, tine_core::PageKind::Page);
        assert_eq!(entry.name, "Renamed Outside");
        assert_eq!(entry.date_key, None);
        assert_eq!(document.roots[0].raw, "after");
    });
    assert!(graph
        .find_entry("Initial Title", tine_core::PageKind::Page)
        .is_none());
    assert!(graph
        .find_entry("Renamed Outside", tine_core::PageKind::Page)
        .is_some());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn malformed_hidden_edn_fails_graph_text_discovery_closed() {
    let root =
        std::env::temp_dir().join(format!("tine-graph-text-hidden-edn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("external")).unwrap();
    std::fs::write(
        root.join("logseq/config.edn"),
        "{:hidden [\"external/private\"}\n",
    )
    .unwrap();
    std::fs::write(root.join("external/Visible.md"), "- must fail closed\n").unwrap();

    let graph = Graph::open(&root);
    assert!(graph.list_pages().is_empty());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn configured_creation_remains_in_configured_page_root() {
    let root = std::env::temp_dir().join(format!("tine-graph-text-create-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"content/pages\"\n\
          :journals-directory \"content/journals\"}\n",
    )
    .unwrap();
    let graph = Graph::open(&root);
    assert!(graph
        .create_markdown_page_if_absent("Created Here", "- created\n")
        .unwrap());
    assert!(root.join("content/pages/Created Here.md").is_file());
    assert!(!root.join("Created Here.md").exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn loads_a_page_with_nesting_and_properties() {
    let g = demo_graph();
    let entry = g
        .find_entry("logseq-claude", tine_core::PageKind::Page)
        .unwrap();
    let dto = g.load_page(&entry).unwrap();
    assert_eq!(
        dto.pre_block.as_deref(),
        Some("title:: logseq-claude\ntags:: project, tooling")
    );
    // Has a nested child under the first block.
    assert!(dto.blocks[0].children.len() >= 1);
}

#[test]
fn backlinks_to_parameterized_complexity() {
    let g = demo_graph();
    let groups = g.backlinks("parameterized complexity");
    let pages: Vec<&str> = groups.iter().map(|gr| gr.page.as_str()).collect();
    // Referenced from the journal, logseq-claude, and n-fold IP.
    assert!(pages.contains(&"logseq-claude"), "pages: {pages:?}");
    assert!(pages.contains(&"n-fold IP"), "pages: {pages:?}");
}

#[test]
fn unlinked_references_include_plain_text_alias_mentions() {
    let root =
        std::env::temp_dir().join(format!("tine-unlinked-alias-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/20260713145345.md"),
        "alias:: 20260713150352\n\n- Alias target page\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/Alias Mention Test.md"),
        "- No matching text yet.\n- The alias is already linked as [[20260713150352]].\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    assert!(
        g.unlinked_refs("20260713145345").is_empty(),
        "warm the derived cache before the source edit"
    );
    let entry = g
        .find_entry("Alias Mention Test", tine_core::PageKind::Page)
        .unwrap();
    let mut page = g.load_page(&entry).unwrap();
    page.blocks[0].raw = "This block mentions 20260713150352 as plain text.".into();
    g.save_page(&page, page.rev.as_deref()).unwrap();
    let groups = g.unlinked_refs("20260713145345");
    assert_eq!(
        groups
            .iter()
            .map(|group| group.page.as_str())
            .collect::<Vec<_>>(),
        vec!["Alias Mention Test"],
        "an alias is another plain-text name for the canonical target: {groups:?}"
    );
    assert_eq!(groups[0].blocks.len(), 1, "linked aliases stay excluded");
    assert_eq!(
        groups[0].blocks[0].raw,
        "This block mentions 20260713150352 as plain text."
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn backlinks_include_explicit_links_in_page_properties() {
    let root = std::env::temp_dir().join(format!(
        "tine-page-property-backlink-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages/A.md"),
        "created:: none\n\n- Page containing a linked reference inside a page property.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals/2026_07_13.md"),
        "- Reference target journal page.\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    assert!(
        g.backlinks("Jul 13th, 2026").is_empty(),
        "warm the derived cache before the page-property edit"
    );
    let entry = g.find_entry("A", tine_core::PageKind::Page).unwrap();
    let mut page = g.load_page(&entry).unwrap();
    page.pre_block = Some("created:: [[Jul 13th, 2026]]".into());
    g.save_page(&page, page.rev.as_deref()).unwrap();
    let groups = g.backlinks("Jul 13th, 2026");
    let source = groups
        .iter()
        .find(|group| group.page == "A")
        .expect("page-property source should be a backlink group");
    assert_eq!(source.blocks.len(), 1, "one page entity counts once");
    assert_eq!(source.blocks[0].raw, "created:: [[Jul 13th, 2026]]");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn backlinks_include_bare_tags_page_properties() {
    let root = std::env::temp_dir().join(format!(
        "tine-page-property-tags-backlink-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/page1.md"), "- Reference target page.\n").unwrap();
    std::fs::write(
        root.join("pages/page2.md"),
        "tags:: unrelated\n\n- Page tagged with the target after load.\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    assert!(
        g.backlinks("page1").is_empty(),
        "warm the derived cache before the bare tags property edit"
    );
    let entry = g.find_entry("page2", tine_core::PageKind::Page).unwrap();
    let mut page = g.load_page(&entry).unwrap();
    page.pre_block = Some("tags:: page1".into());
    g.save_page(&page, page.rev.as_deref()).unwrap();

    let groups = g.backlinks("page1");
    let source = groups
        .iter()
        .find(|group| group.page == "page2")
        .expect("a bare tags:: value should create a Linked Reference group");
    assert_eq!(source.blocks.len(), 1, "one property source counts once");
    assert_eq!(source.blocks[0].raw, "tags:: page1");
    assert!(source.blocks[0].page_property);
    assert_eq!(source.evidence.len(), 1);
    assert_eq!(source.evidence[0].occurrences.len(), 1);
    let occurrence = &source.evidence[0].occurrences[0];
    assert_eq!(occurrence.matched_name, "page1");
    assert_eq!(occurrence.canonical, "page1");
    assert_eq!(occurrence.span.start, "tags:: ".encode_utf16().count());
    assert_eq!(occurrence.span.end, "tags:: page1".encode_utf16().count());
    assert_eq!(occurrence.rule, "implicit_linkable_property");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn block_ref_counts_and_referrers() {
    // Isolated temp graph: a target block (id:: aaaaaaaa-0000-0000-0000-000000000001) referenced by a same-page
    // block and three blocks on another page (labeled, embed, and a double ref that
    // must dedupe to one). Exercises all three OG block-ref forms + same-page
    // inclusion.
    let root = std::env::temp_dir().join(format!("tine-blockref-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages").join("Target.md"),
        "- the target block\n  id:: aaaaaaaa-0000-0000-0000-000000000001\n- see ((aaaaaaaa-0000-0000-0000-000000000001)) on this page\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Other.md"),
        "- ref via [label](((aaaaaaaa-0000-0000-0000-000000000001)))\n- embedded {{embed ((aaaaaaaa-0000-0000-0000-000000000001))}}\n- two ((aaaaaaaa-0000-0000-0000-000000000001)) and ((aaaaaaaa-0000-0000-0000-000000000001)) here\n",
    )
    .unwrap();

    let g = Graph::open(&root);

    // Count = distinct referrer blocks: 1 (same page) + 3 (Other) = 4. The double
    // ref on the last Other block counts once.
    let counts = g.block_ref_counts().unwrap();
    assert_eq!(
        counts.get("aaaaaaaa-0000-0000-0000-000000000001").copied(),
        Some(4),
        "counts: {counts:?}"
    );

    // Referrers grouped by page, and the same-page referrer IS included (unlike
    // page backlinks).
    let groups = g.block_referrers("aaaaaaaa-0000-0000-0000-000000000001");
    let mut by_page: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for gr in groups.iter() {
        by_page.insert(gr.page.as_str(), gr.blocks.len());
    }
    assert_eq!(
        by_page.get("Target").copied(),
        Some(1),
        "same-page referrer included"
    );
    assert_eq!(
        by_page.get("Other").copied(),
        Some(3),
        "all 3 Other referrers"
    );

    // The target block itself is not a referrer of itself.
    let target_refs: Vec<&str> = groups
        .iter()
        .find(|gr| gr.page == "Target")
        .map(|gr| gr.blocks.iter().map(|b| b.raw.as_str()).collect())
        .unwrap_or_default();
    assert!(
        target_refs
            .iter()
            .all(|r| r.contains("see ((aaaaaaaa-0000-0000-0000-000000000001))")),
        "{target_refs:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn publishes_only_public_pages() {
    let root = std::env::temp_dir().join(format!("tine-publish-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages").join("Shared.md"),
        "public:: true\n\n- hello, see [[Secret]]\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("Secret.md"), "- private notes\n").unwrap();

    let g = Graph::open(&root);
    let (dir, n) = g.publish_html().unwrap();
    assert_eq!(n, 1, "only the public page is published");
    let p = std::fs::read_to_string(format!("{dir}/shared.html")).unwrap();
    assert!(p.contains("<h1 class=\"page\">Shared</h1>"));
    assert!(p.contains("<a class=\"ref\""), "should link [[refs]]");
    // The private page must not be exported.
    assert!(!std::path::Path::new(&format!("{dir}/secret.html")).exists());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_cache_reflects_saves_and_deletes() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    // Isolated temp graph so we can mutate it freely.
    let root = std::env::temp_dir().join(format!("tine-cache-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages").join("Seed.md"), "- a seed block\n").unwrap();

    let g = Graph::open(&root);
    // Warms the cache on first search.
    assert_eq!(g.search("zonkwort", 10).len(), 0, "token absent initially");

    // Saving a page with the token must be visible to a subsequent search
    // without any disk re-scan (cache upsert).
    let page = PageDto {
        name: "Fresh".into(),
        kind: PageKind::Page,
        title: "Fresh".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "contains zonkwort here".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        activation: None,
        guide: false,
    };
    g.save_page(&page, None).unwrap();
    let hits = g.search("zonkwort", 10);
    assert_eq!(hits.len(), 1, "saved page should be searchable");
    assert_eq!(hits[0].page, "Fresh");

    // Deleting the page removes it from the cache too.
    g.delete_page("Fresh", PageKind::Page).unwrap();
    assert_eq!(
        g.search("zonkwort", 10).len(),
        0,
        "deleted page should drop out"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn journal_template_bytes_survive_reopen_and_idempotent_resave() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!(
        "tine-journal-template-reopen-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let graph = Graph::open(&root);
    let page = PageDto {
        name: "Jul 30th, 2026".into(),
        kind: PageKind::Journal,
        title: "Jul 30th, 2026".into(),
        pre_block: None,
        blocks: ["### Meetings", "### Notes", "### Tasks"]
            .into_iter()
            .map(|raw| BlockDto {
                raw: raw.into(),
                ..Default::default()
            })
            .collect(),
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        activation: None,
        guide: false,
    };

    graph.save_page(&page, None).unwrap();
    let journal_path = root.join("journals/2026_07_30.md");
    let expected = b"- ### Meetings\n- ### Notes\n- ### Tasks\n";
    assert_eq!(std::fs::read(&journal_path).unwrap(), expected);
    drop(graph);

    let reopened = Graph::open(&root);
    let entry = reopened
        .find_entry("Jul 30th, 2026", PageKind::Journal)
        .expect("templated journal must be indexed after restart");
    let loaded = reopened.load_page(&entry).unwrap();
    assert_eq!(
        loaded
            .blocks
            .iter()
            .map(|block| block.raw.as_str())
            .collect::<Vec<_>>(),
        vec!["### Meetings", "### Notes", "### Tasks"]
    );
    reopened.save_page(&loaded, loaded.rev.as_deref()).unwrap();
    assert_eq!(std::fs::read(&journal_path).unwrap(), expected);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_ignores_hidden_property_metadata() {
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-search-meta-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);

    // A block whose only occurrence of "qzxmeta" is in a property line (like an
    // id:: uuid or hl-color::) must NOT match — the user can't see it.
    let page = PageDto {
        name: "Meta".into(),
        kind: PageKind::Page,
        title: "Meta".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "a perfectly ordinary block\nsome-prop:: qzxmeta".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        activation: None,
        guide: false,
    };
    g.save_page(&page, None).unwrap();
    assert_eq!(
        g.search("qzxmeta", 10).len(),
        0,
        "token only in a property line should not be a search hit"
    );
    // But the visible body is still searchable.
    assert_eq!(
        g.search("ordinary", 10).len(),
        1,
        "visible body still matches"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_preserves_file_format_no_churn() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-fmt-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Logseq style: no trailing newline. Plus one with a newline, and a
    // space-indented file (Tine emits tabs by default — must preserve spaces).
    let no_nl = "- alpha\n\t- beta";
    let with_nl = "- gamma\n";
    let spaces = "- root\n  - two-space child\n    - grandchild";
    std::fs::write(root.join("pages").join("A.md"), no_nl).unwrap();
    std::fs::write(root.join("pages").join("B.md"), with_nl).unwrap();
    std::fs::write(root.join("pages").join("C.md"), spaces).unwrap();

    let g = Graph::open(&root);
    // Load then save unchanged must be byte-identical (no churn): each file's
    // trailing-newline + indent convention is preserved.
    for name in ["A", "B", "C"] {
        let dto = g.load_named(name, PageKind::Page).unwrap().unwrap();
        g.save_page(&dto, dto.rev.as_deref()).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("A.md")).unwrap(),
        no_nl
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("B.md")).unwrap(),
        with_nl
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("C.md")).unwrap(),
        spaces
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn sheet_field_rename_org_saves_and_reparses_every_dependency() {
    use tine_core::model::{Format, PageKind};

    let root = std::env::temp_dir().join(format!(
        "tine-sheet-field-rename-org-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    let path = root.join("pages").join("Sheet.org");
    let before = [
        "* Table",
        ":PROPERTIES:",
        ":tine.view: table",
        ":tine.fields: severity=number;occurrence=number;detection=number",
        ":tine.formula.rpn: severity * occurrence * detection + if(label == \"occurrence\", formula.occurrence, 0)",
        ":tine.filter: occurrence > 1",
        ":tine.group-by: prop:occurrence",
        ":tine.col-aggregates: prop:occurrence=sum;prop:severity=max",
        ":END:",
        "** Row",
        ":PROPERTIES:",
        ":severity: 2",
        ":occurrence: 2",
        ":detection: 2",
        ":label: other",
        ":END:",
        "",
    ]
    .join("\n");
    std::fs::write(&path, &before).unwrap();

    // This DTO is the exact write shape produced by the frontend's already-unit-
    // tested rename plan. Exercise the real guarded Graph save, inspect bytes,
    // then construct a fresh Graph so this cannot pass on an in-memory document.
    let graph = Graph::open(&root);
    let mut page = graph
        .load_named("Sheet", PageKind::Page)
        .unwrap()
        .expect("org sheet");
    assert_eq!(page.format, Format::Org);
    assert!(!page.read_only, "canonical Org fixture must be writable");
    let owner = &mut page.blocks[0];
    owner.raw = owner
        .raw
        .replace(
            ":tine.fields: severity=number;occurrence=number;detection=number",
            ":tine.fields: severity=number;OCC=number;detection=number",
        )
        .replace(
            ":tine.formula.rpn: severity * occurrence * detection + if(label == \"occurrence\", formula.occurrence, 0)",
            ":tine.formula.rpn: severity * OCC * detection + if(label == \"occurrence\", formula.occurrence, 0)",
        )
        .replace(":tine.filter: occurrence > 1", ":tine.filter: OCC > 1")
        .replace(
            ":tine.group-by: prop:occurrence",
            ":tine.group-by: prop:OCC",
        )
        .replace(
            ":tine.col-aggregates: prop:occurrence=sum;prop:severity=max",
            ":tine.col-aggregates: prop:OCC=sum;prop:severity=max",
        );
    owner.children[0].raw = owner.children[0].raw.replace(":occurrence: 2", ":OCC: 2");
    graph
        .save_page(&page, page.rev.as_deref())
        .expect("guarded Org save");

    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(disk.contains(":tine.fields: severity=number;OCC=number;detection=number"));
    assert!(disk.contains(
        ":tine.formula.rpn: severity * OCC * detection + if(label == \"occurrence\", formula.occurrence, 0)"
    ));
    assert!(disk.contains(":tine.filter: OCC > 1"));
    assert!(disk.contains(":tine.group-by: prop:OCC"));
    assert!(disk.contains(":tine.col-aggregates: prop:OCC=sum;prop:severity=max"));
    assert!(disk.contains(":OCC: 2"));
    assert!(!disk.contains("occurrence=number"));
    assert!(!disk.contains("severity * occurrence * detection"));
    assert!(!disk.contains(":tine.filter: occurrence > 1"));
    assert!(!disk.contains("prop:occurrence"));
    assert!(!disk.contains(":occurrence: 2"));
    assert!(
        disk.contains("if(label == \"occurrence\", formula.occurrence, 0)"),
        "string literal and formula member are not field identities"
    );

    let reopened = Graph::open(&root)
        .load_named("Sheet", PageKind::Page)
        .unwrap()
        .expect("reparsed org sheet");
    assert_eq!(reopened.format, Format::Org);
    assert!(
        !reopened.read_only,
        "renamed Org bytes must remain writable"
    );
    assert_eq!(reopened.blocks[0].raw, page.blocks[0].raw);
    assert_eq!(
        reopened.blocks[0].children[0].raw,
        page.blocks[0].children[0].raw
    );
    assert!(
        reopened.blocks[0]
            .properties
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("tine.fields")
                && value.contains("OCC=number")),
        "fresh parser recognizes the renamed schema"
    );
    assert!(
        reopened.blocks[0].children[0]
            .properties
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("OCC") && value == "2"),
        "fresh parser recognizes the renamed row property"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_refuses_to_clobber_external_change() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-conflict-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();

    let g = Graph::open(&root);
    // Build the cache (Tine now "knows" N = "- one"), then load it for editing.
    g.search("one", 10);
    let mut dto = g.load_named("N", PageKind::Page).unwrap().unwrap();
    as_editor(&g, &mut dto);

    // An external writer (another app / Syncthing) changes the file.
    std::fs::write(&path, "- EXTERNAL EDIT").unwrap();

    // Saving the now-stale page must fail with a conflict and NOT overwrite.
    let err = g.save_page(&dto, dto.rev.as_deref()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- EXTERNAL EDIT");

    // "Keep mine" force-saves over it.
    let shown = g
        .outstanding_conflict_override(&dto)
        .unwrap()
        .expect("the refused save names the conflict shown to this editor");
    g.force_save_page_at_revision(&dto, dto.rev.as_deref(), shown)
        .unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("one"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn save_conflicts_when_file_deleted_externally() {
    use tine_core::model::PageKind;
    let root = std::env::temp_dir().join(format!("tine-del-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();
    let g = Graph::open(&root);
    g.search("one", 10); // warm cache
    let dto = g.load_named("N", PageKind::Page).unwrap().unwrap();

    // The file is deleted on disk (Syncthing / Logseq) after we loaded it.
    std::fs::remove_file(&path).unwrap();

    // Saving must conflict, NOT silently resurrect the deleted note.
    let err = g.save_page(&dto, dto.rev.as_deref()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(
        !path.exists(),
        "deleted file must stay deleted on a conflicting save"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn load_reflects_external_change_then_save_is_clean() {
    use tine_core::model::PageKind;
    let root = std::env::temp_dir().join(format!("tine-reconcile-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("N.md");
    std::fs::write(&path, "- one").unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let _ = g.load_named("N", PageKind::Page).unwrap().unwrap(); // cache built

    // External writer changes the file; the 3s watcher hasn't run yet.
    std::fs::write(&path, "- TWO external").unwrap();

    // load_page must reconcile and serve the NEW content (not the stale cache),
    // and the rev it returns must match disk so a save doesn't spuriously conflict.
    let dto = g.load_named("N", PageKind::Page).unwrap().unwrap();
    assert!(
        dto.blocks[0].raw.contains("TWO external"),
        "load reflects external change"
    );
    g.save_page(&dto, dto.rev.as_deref())
        .expect("save of freshly-loaded current content is clean");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn consecutive_self_saves_do_not_conflict() {
    // Regression: inserting a SCHEDULED date then deleting it (consecutive saves
    // with no external writer) must not raise a spurious "changed on disk"
    // conflict. Each save returns the new baseline rev, which the next save passes
    // back (exactly what the frontend does) — so our own write is never mistaken
    // for an external edit.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-selfsave-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    let mk = |raw: &str| PageDto {
        name: "D".into(),
        kind: PageKind::Page,
        title: "D".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        activation: None,
        guide: false,
    };
    // 1) date picker inserts a SCHEDULED line (page is new — no baseline yet).
    let r1 = g
        .save_page(&mk("TODO task\nSCHEDULED: <2026-06-16 Tue>"), None)
        .unwrap();
    // 2) user deletes the inserted text — must NOT be read as an external edit.
    let r2 = g
        .save_page(&mk("TODO task"), Some(&r1))
        .expect("no spurious conflict after our own save");
    // 3) and a further edit still saves cleanly.
    g.save_page(&mk("TODO task edited"), Some(&r2))
        .expect("no spurious conflict");
    assert!(std::fs::read_to_string(root.join("pages").join("D.md"))
        .unwrap()
        .contains("edited"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn sync_file_detects_external_change_and_suppresses_self() {
    use tine_core::model::PageKind;

    let root = std::env::temp_dir().join(format!("tine-sync-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages").join("S.md");
    std::fs::write(&path, "- before").unwrap();

    let g = Graph::open(&root);
    g.search("before", 10); // build the cache (S = "- before")

    // No external change yet → sync reports nothing.
    assert!(g.sync_file(&path).is_none());

    // External edit → sync reports the entry and refreshes the cache.
    std::fs::write(&path, "- after the change").unwrap();
    let changed = g.sync_file(&path).expect("external change detected");
    assert_eq!(changed.name, "S");
    assert_eq!(changed.kind, PageKind::Page);
    assert_eq!(
        g.search("after", 10).len(),
        1,
        "cache updated to new content"
    );
    assert_eq!(g.search("before", 10).len(), 0);

    // Re-syncing the same content is a no-op (self-write suppression).
    assert!(g.sync_file(&path).is_none());

    // Deletion is reported and drops it from the cache.
    std::fs::remove_file(&path).unwrap();
    assert!(g.forget_file(&path).is_some());
    assert_eq!(g.search("after", 10).len(), 0);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn noop_save_does_not_bump_cache_generation() {
    // A save whose serialized bytes match disk (focus/blur, forced flush of an
    // unchanged page) must NOT bump cache_gen — that key invalidates every
    // memoized query/backlink/derived result, so a no-op re-save would force a
    // whole-graph requery on every open dashboard. A real edit still bumps it.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-noopgen-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    g.search("x", 10); // build the cache
    let mk = |raw: &str| PageDto {
        name: "N".into(),
        kind: PageKind::Page,
        title: "N".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        activation: None,
        guide: false,
    };
    let r1 = g.save_page(&mk("hello"), None).unwrap();
    let gen1 = g.cache_generation();
    // Re-save byte-identical content (no-op) with the returned baseline.
    let r2 = g.save_page(&mk("hello"), Some(&r1)).unwrap();
    assert_eq!(r1, r2, "rev must be stable across a no-op save");
    assert_eq!(
        g.cache_generation(),
        gen1,
        "no-op save must not bump cache_gen"
    );
    // A real edit DOES bump it.
    g.save_page(&mk("hello world"), Some(&r2)).unwrap();
    assert!(
        g.cache_generation() > gen1,
        "a real edit must bump cache_gen"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn self_write_marker_does_not_outlive_its_save() {
    // The self-write marker only covers the rename→cache_upsert window and is
    // dropped by the writer once the write is published, so it can't linger and
    // later suppress a REAL external change that restores Tine's earlier bytes
    // (a delete+recreate, here simulated by forgetting the cached page and
    // re-syncing the still-on-disk file). Before this fix, the stale marker made
    // the recreate look like our own write and it was silently dropped.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-marker-life-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    g.search("x", 10);
    let page = PageDto {
        name: "C".into(),
        kind: PageKind::Page,
        title: "C".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: "noted".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        activation: None,
        guide: false,
    };
    g.save_page(&page, None).unwrap(); // sets, then self-removes, the marker
    let path = root.join("pages").join("C.md");
    assert!(
        g.forget_file(&path).is_some(),
        "page should have been cached"
    );
    // The page still exists on disk; with the marker gone, re-syncing must treat
    // it as a real (re)appearance, not a suppressed self-write.
    assert!(
        g.sync_file(&path).is_some(),
        "a stale self-write marker must not suppress the page reappearing"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn disk_rev_fast_path_is_fresh_and_detects_external_change() {
    // B1: an unchanged file syncs to a no-op via the disk_rev fast-path (no
    // reparse), but a genuine external edit is still detected — the rev must
    // never mask a change, and the served content must update.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-diskrev-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    g.search("x", 10); // build the cache
    let page = PageDto {
        name: "R".into(),
        kind: PageKind::Page,
        title: "R".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: "alpha".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        activation: None,
        guide: false,
    };
    g.save_page(&page, None).unwrap(); // populates disk_revs[R] (marker self-removed)
    let path = root.join("pages").join("R.md");
    assert!(
        g.sync_file(&path).is_none(),
        "unchanged save → suppressed via disk_rev fast-path"
    );
    assert!(
        g.sync_file(&path).is_none(),
        "still unchanged → disk_rev fast-path again"
    );

    // A real external edit must still be detected (not masked by disk_revs).
    std::fs::write(&path, "- beta\n").unwrap();
    let changed = g
        .sync_file(&path)
        .expect("external change detected despite disk_revs entry");
    assert_eq!(changed.name, "R");
    // The cache now serves the new content (load_page goes through sync_file_content).
    let dto = g.load_named("R", PageKind::Page).unwrap().unwrap();
    assert!(
        dto.blocks.iter().any(|b| b.raw.contains("beta")),
        "served stale after external edit"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn self_write_is_not_reported_as_external_change() {
    // Regression for the false "changed on disk" conflict seen during normal
    // typing (no external writer): a watcher poll after Tine's own save must not
    // report a false external change. Post-save, disk_revs reflects the write and
    // suppresses the poll (the short-lived marker covers only the in-flight
    // window). Uses the multi-line `> quote` shape that surfaced the original bug.
    use tine_core::model::{BlockDto, PageDto, PageKind};

    let root = std::env::temp_dir().join(format!("tine-selfwrite-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let g = Graph::open(&root);
    g.search("x", 10); // build the cache

    let path = root.join("pages").join("W.md");
    let page = PageDto {
        name: "W".into(),
        kind: PageKind::Page,
        title: "W".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b1".into(),
            raw: "hello\n> quote".into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        path: String::new(),
        activation: None,
        guide: false,
    };
    g.save_page(&page, None).unwrap();

    // A watcher poll right after our own save must emit nothing.
    assert!(
        g.sync_file(&path).is_none(),
        "Tine's own save must not be reported as an external change"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn query_between_filters_by_journal_date() {
    let root = std::env::temp_dir().join(format!("tine-between-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("journals").join("2022_06_15.md"),
        "- TODO [[scs]] recent\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2019_01_01.md"),
        "- TODO [[scs]] old\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    let groups = g
        .run_query("(and (task TODO) (and [[scs]] (between [[Jan 1st, 2021]] [[Jan 1st, 2100]])))");
    let raws: Vec<String> = groups
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(
        raws.iter().any(|r| r.contains("recent")),
        "in-range journal matches: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("old")),
        "out-of-range journal excluded: {raws:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_page_moves_file_and_updates_refs() {
    let root = std::env::temp_dir().join(format!("tine-rename-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Old Name.md"), "- the page body\n").unwrap();
    std::fs::write(
        root.join("pages").join("Other.md"),
        "- see [[Old Name]] and #[[Old Name]]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_15.md"),
        "- ref [[Old Name]]\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    g.rename_page("Old Name", "New Name").unwrap();

    // File moved.
    assert!(!root.join("pages").join("Old Name.md").exists());
    assert!(root.join("pages").join("New Name.md").exists());
    // References rewritten everywhere.
    let other = std::fs::read_to_string(root.join("pages").join("Other.md")).unwrap();
    assert!(other.contains("[[New Name]]"), "{other}");
    assert!(other.contains("#[[New Name]]"), "{other}");
    assert!(!other.contains("Old Name"), "{other}");
    let journal = std::fs::read_to_string(root.join("journals").join("2026_06_15.md")).unwrap();
    assert!(journal.contains("[[New Name]]"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_cascades_namespace_and_rewrites_self_refs() {
    // F2 (namespace cascade) + F3 (self-refs): renaming `Proj` must move every
    // `Proj/*` page to `Renamed/*`, rewrite refs to all of them everywhere, and
    // rewrite the renamed pages' OWN refs.
    let root = std::env::temp_dir().join(format!("tine-ns-rename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    // These namespace pages use the `___` separator, so the graph must be pinned
    // to :triple-lowbar (the modern format) — otherwise (legacy default) `___`
    // isn't a separator and the cascade wouldn't see them as `Proj/*` children.
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Proj.md"),
        "- root, see [[Proj]] self\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Proj___Child.md"),
        "- child of [[Proj]]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Proj___Child___Deep.md"),
        "- deep\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages").join("Other.md"),
        "- [[Proj]] and [[Proj/Child]]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_15.md"),
        "- note [[Proj/Child/Deep]]\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    g.rename_page("Proj", "Renamed").unwrap();

    let p = root.join("pages");
    // Subtree moved.
    assert!(!p.join("Proj.md").exists());
    assert!(!p.join("Proj___Child.md").exists());
    assert!(!p.join("Proj___Child___Deep.md").exists());
    assert!(p.join("Renamed.md").exists());
    assert!(p.join("Renamed___Child.md").exists());
    assert!(p.join("Renamed___Child___Deep.md").exists());
    // F3: the renamed page's own self-ref rewritten.
    let renamed = std::fs::read_to_string(p.join("Renamed.md")).unwrap();
    assert!(renamed.contains("[[Renamed]]"), "self-ref: {renamed}");
    // Child's ref to its parent rewritten.
    let child = std::fs::read_to_string(p.join("Renamed___Child.md")).unwrap();
    assert!(child.contains("[[Renamed]]"), "child→parent ref: {child}");
    // Refs everywhere rewritten, parent and namespaced child both.
    let other = std::fs::read_to_string(p.join("Other.md")).unwrap();
    assert!(
        other.contains("[[Renamed]]") && other.contains("[[Renamed/Child]]"),
        "{other}"
    );
    assert!(!other.contains("Proj"), "{other}");
    let journal = std::fs::read_to_string(root.join("journals").join("2026_06_15.md")).unwrap();
    assert!(journal.contains("[[Renamed/Child/Deep]]"), "{journal}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn legacy_namespace_file_round_trips() {
    // No :file/name-format ⇒ legacy (OG's default). A `%2F`-encoded namespace
    // file (what OG writes on a legacy graph) must be discoverable under its
    // slashed name and save back to the SAME file — never silently forked into a
    // `___` twin. This is the G1/G2 fix: Tine used to always use/expect `___`.
    let root = std::env::temp_dir().join(format!("tine-ns-legacy-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages").join("math%2Falgebra.md"),
        "- legacy ns page\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    // Discoverable under the decoded, slashed name.
    let entry = g
        .find_entry("math/algebra", tine_core::PageKind::Page)
        .expect("legacy %2F file should resolve under its slashed name");
    assert_eq!(entry.name, "math/algebra");
    let dto = g.load_page(&entry).unwrap();
    // An edited save round-trips to the SAME file; no `___` twin appears.
    g.save_page(&dto, dto.rev.as_deref()).unwrap();
    assert!(root.join("pages").join("math%2Falgebra.md").exists());
    assert!(
        !root.join("pages").join("math___algebra.md").exists(),
        "must not fork a triple-lowbar twin"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_rewrites_bare_tags_property() {
    // F1: a page referencing the renamed page only through a bare `tags::` value
    // (no inline [[..]]) must have that value rewritten; siblings preserved.
    let root = std::env::temp_dir().join(format!("tine-tags-rename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Old.md"), "- old page body\n").unwrap();
    std::fs::write(
        root.join("pages").join("Note.md"),
        "tags:: Old, keep\n\n- body\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    g.rename_page("Old", "New").unwrap();

    assert!(!root.join("pages").join("Old.md").exists());
    assert!(root.join("pages").join("New.md").exists());
    let note = std::fs::read_to_string(root.join("pages").join("Note.md")).unwrap();
    assert!(
        note.contains("tags:: New, keep"),
        "bare tag rewritten + sibling kept: {note}"
    );
    assert!(!note.contains("Old"), "{note}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_aborts_on_target_collision_without_changes() {
    // Renaming onto an existing page name aborts with NO change (Tine doesn't
    // merge). Both files must be byte-identical afterwards.
    let root = std::env::temp_dir().join(format!("tine-collide-rename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("A.md"), "- a body [[B]]\n").unwrap();
    std::fs::write(root.join("pages").join("B.md"), "- b body\n").unwrap();

    let g = Graph::open(&root);
    assert!(
        g.rename_page("A", "B").is_err(),
        "rename onto existing page must fail"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("A.md")).unwrap(),
        "- a body [[B]]\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pages").join("B.md")).unwrap(),
        "- b body\n"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_ref_only_page_rewrites_refs_without_a_file() {
    // A page that exists only via references (no file of its own) still has its
    // refs rewritten across the graph.
    let root = std::env::temp_dir().join(format!("tine-refonly-rename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("pages").join("Ref.md"),
        "- mentions [[Ghost]] here\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    g.rename_page("Ghost", "Spirit").unwrap();

    assert!(!root.join("pages").join("Ghost.md").exists());
    let r = std::fs::read_to_string(root.join("pages").join("Ref.md")).unwrap();
    assert!(r.contains("[[Spirit]]") && !r.contains("Ghost"), "{r}");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn query_and_not_includes_everything_except_excluded() {
    // (and (task TODO) (not [[X]])) must return ALL TODO blocks that don't
    // reference [[X]] — regression for "NOT excludes right but drops others".
    let root = std::env::temp_dir().join(format!("tine-not-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("pages").join("P.md"),
        "- TODO alpha\n- TODO beta [[X]]\n- TODO gamma\n- DONE delta\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    let raws: Vec<String> = g
        .run_query("(and (task TODO) (not [[X]]))")
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(raws.iter().any(|r| r.contains("alpha")), "{raws:?}");
    assert!(raws.iter().any(|r| r.contains("gamma")), "{raws:?}");
    assert!(
        !raws.iter().any(|r| r.contains("beta")),
        "X-referencing excluded: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("delta")),
        "non-TODO excluded: {raws:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn page_aliases_resolve_and_collect_backlinks() {
    use tine_core::model::PageKind;
    let root = std::env::temp_dir().join(format!("tine-alias-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Canonical page "Parameterized Complexity" with an alias "PC".
    std::fs::write(
        root.join("pages").join("Parameterized Complexity.md"),
        "alias:: PC\n\n- the canonical page\n",
    )
    .unwrap();
    // One page links the canonical name, another links the alias.
    std::fs::write(
        root.join("pages").join("A.md"),
        "- see [[Parameterized Complexity]]\n",
    )
    .unwrap();
    std::fs::write(root.join("pages").join("B.md"), "- via [[PC]]\n").unwrap();

    let g = Graph::open(&root);
    // Loading the alias resolves to the canonical page.
    let dto = g
        .load_named("PC", PageKind::Page)
        .unwrap()
        .expect("alias resolves");
    assert!(dto.blocks.iter().any(|b| b.raw.contains("canonical page")));
    // Backlinks of the canonical page include the alias-referencing page.
    let pages: Vec<String> = g
        .backlinks("Parameterized Complexity")
        .iter()
        .map(|gr| gr.page.clone())
        .collect();
    assert!(pages.contains(&"A".to_string()), "{pages:?}");
    assert!(
        pages.contains(&"B".to_string()),
        "alias ref counted: {pages:?}"
    );
    // Backlinks queried via the alias name also resolve to the canonical set.
    let via_alias: Vec<String> = g.backlinks("PC").iter().map(|gr| gr.page.clone()).collect();
    assert!(
        via_alias.contains(&"A".to_string()) && via_alias.contains(&"B".to_string()),
        "{via_alias:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_favorites_round_trips_in_config_edn() {
    let root = std::env::temp_dir().join(format!("tine-fav-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Existing config with another key that must be preserved + a COMMENTED
    // :favorites decoy that must NOT be edited (the write must target the real key).
    std::fs::write(root.join("logseq").join("config.edn"),
        "{;; :favorites [\"example\"]\n :preferred-workflow :now\n :journals-directory \"journals\"}\n").unwrap();

    let g = Graph::open(&root);
    g.set_favorites(&["Inbox".into(), "Reading List".into()])
        .unwrap();
    // Re-open and confirm favorites parsed back + the other key survived.
    let g2 = Graph::open(&root);
    assert_eq!(
        g2.meta().favorites,
        vec!["Inbox".to_string(), "Reading List".to_string()]
    );
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert!(
        cfg.contains(":journals-directory"),
        "other keys preserved: {cfg}"
    );
    assert!(
        cfg.contains(";; :favorites [\"example\"]"),
        "commented decoy untouched: {cfg}"
    );

    // Updating again replaces (not appends) the vector.
    g2.set_favorites(&["Only One".into()]).unwrap();
    assert_eq!(
        Graph::open(&root).meta().favorites,
        vec!["Only One".to_string()]
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_favorites_edn_aware_vector_end() {
    // Round-6 audit: the end of `:favorites [...]` was found with the first raw
    // `]`, so a favorite NAME containing `]` truncated the replacement and left a
    // corrupt fragment. The end scan is now EDN-aware (skips strings/escapes).
    let root = std::env::temp_dir().join(format!("tine-fav-edn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    // Existing vector whose FIRST entry contains a `]`, plus a sibling key.
    std::fs::write(
        &cfg_path,
        "{:favorites [\"A]B\" \"C\"]\n :journals-directory \"journals\"}\n",
    )
    .unwrap();

    Graph::open(&root).set_favorites(&["Only".into()]).unwrap();
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    assert_eq!(
        Graph::open(&root).meta().favorites,
        vec!["Only".to_string()]
    );
    assert!(
        cfg.contains(":journals-directory"),
        "sibling preserved: {cfg}"
    );
    // The whole old vector is gone — no truncation fragment left behind.
    assert!(
        !cfg.contains("A]B"),
        "old first entry fully replaced: {cfg}"
    );
    assert!(
        !cfg.contains("\"C\""),
        "old second entry gone (no leftover): {cfg}"
    );
    assert_eq!(
        cfg.matches(":favorites").count(),
        1,
        "exactly one :favorites: {cfg}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_preferred_workflow_ignores_key_inside_string_literal() {
    // Round-7 audit: the key was located with a non-string-aware scan, so a
    // `:preferred-workflow` inside a string value could be edited instead of the
    // real key. `find_keyword` now skips strings.
    let root = std::env::temp_dir().join(format!("tine-wf-str-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    // A string value mentions the key; the REAL key is separate and says :now.
    std::fs::write(
        &cfg_path,
        "{:note \":preferred-workflow :now\"\n :preferred-workflow :now}\n",
    )
    .unwrap();

    Graph::open(&root).set_preferred_workflow("todo").unwrap();
    let c = std::fs::read_to_string(&cfg_path).unwrap();
    // The string decoy is untouched; the REAL (non-string) key flipped to :todo.
    // (Asserted on file content: the matching READER `keyword_value` isn't
    // string-aware for key LOCATION — a separate, far more pathological case
    // codex did not flag — so we verify the writer targeted the right key here.)
    assert!(
        c.contains("\":preferred-workflow :now\""),
        "string decoy untouched: {c}"
    );
    assert!(
        c.contains(":preferred-workflow :todo"),
        "real key flipped in file: {c}"
    );
    assert_eq!(
        c.matches(":todo").count(),
        1,
        "exactly the real value changed: {c}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn config_readers_are_edn_aware_for_bracket_brace_in_values() {
    // Round-7 audit: the hardened writers can now emit a `]`/`}` inside a string
    // value (a favorited/templated page titled `f[x]` / `Plan {B}`); the readers
    // used delimiter scans that truncated at the first raw `]`/`}` and silently
    // lost the value on reload. Writer→file→reader must now round-trip.
    let root = std::env::temp_dir().join(format!("tine-cfg-read-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();

    // Favorites with a `]` in a name (and a plain sibling) survive a reload.
    Graph::open(&root)
        .set_favorites(&["arr[0]".into(), "plain".into()])
        .unwrap();
    assert_eq!(
        Graph::open(&root).meta().favorites,
        vec!["arr[0]".to_string(), "plain".to_string()]
    );

    // Default journal template name containing `}` survives a reload.
    Graph::open(&root)
        .set_default_journal_template(Some("Plan {B}"))
        .unwrap();
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("Plan {B}")
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn config_reader_preserves_semicolon_in_string_values() {
    // Round-8 audit: comment stripping cut every line at the first `;`, even
    // inside a string, so a favorited/templated page named `A;B` reloaded as `A`.
    // Comment stripping is now string-aware.
    let root = std::env::temp_dir().join(format!("tine-cfg-semi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();

    Graph::open(&root)
        .set_favorites(&["A;B".into(), "C".into()])
        .unwrap();
    assert_eq!(
        Graph::open(&root).meta().favorites,
        vec!["A;B".to_string(), "C".to_string()]
    );

    Graph::open(&root)
        .set_default_journal_template(Some("Plan;B"))
        .unwrap();
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("Plan;B")
    );

    // A real (line-start) comment is still stripped — the decoy must not be read.
    let cfg_path = root.join("logseq").join("config.edn");
    let cur = std::fs::read_to_string(&cfg_path).unwrap();
    std::fs::write(
        &cfg_path,
        format!(";; :journals-directory \"DECOY\"\n{cur}"),
    )
    .unwrap();
    assert_ne!(
        Graph::open(&root).meta().journals_dir,
        "DECOY",
        "line comment still stripped"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn editor_structure_preferences_round_trip_through_config_edn() {
    let root = std::env::temp_dir().join(format!(
        "tine-editor-structure-config-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let config = root.join("logseq").join("config.edn");
    std::fs::write(&config, "{:unrelated true}\n").unwrap();

    let graph = Graph::open(&root);
    assert!(!graph.meta().doc_mode_enter_for_new_block);
    assert!(!graph.meta().logical_outdenting);
    graph.set_doc_mode_enter_for_new_block(true).unwrap();
    graph.set_logical_outdenting(true).unwrap();

    let reopened = Graph::open(&root);
    assert!(reopened.meta().doc_mode_enter_for_new_block);
    assert!(reopened.meta().logical_outdenting);
    let persisted = std::fs::read_to_string(&config).unwrap();
    assert!(persisted.contains(":shortcut/doc-mode-enter-for-new-block? true"));
    assert!(persisted.contains(":editor/logical-outdenting? true"));
    assert!(persisted.contains(":unrelated true"));

    reopened.set_doc_mode_enter_for_new_block(false).unwrap();
    reopened.set_logical_outdenting(false).unwrap();
    let reopened_again = Graph::open(&root);
    assert!(!reopened_again.meta().doc_mode_enter_for_new_block);
    assert!(!reopened_again.meta().logical_outdenting);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_preferred_workflow_round_trips_in_config_edn() {
    let root = std::env::temp_dir().join(format!("tine-wf-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // Existing config: another key + a *commented* decoy that must NOT be edited.
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{;; :preferred-workflow :todo (example)\n :preferred-workflow :now\n :start-of-week 1}\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    assert_eq!(g.meta().preferred_workflow, "now");
    g.set_preferred_workflow("todo").unwrap();

    let g2 = Graph::open(&root);
    assert_eq!(g2.meta().preferred_workflow, "todo");
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert!(
        cfg.contains(":start-of-week 1"),
        "other keys preserved: {cfg}"
    );
    // The commented decoy line is untouched (still says :todo (example)).
    assert!(
        cfg.contains(";; :preferred-workflow :todo (example)"),
        "comment preserved: {cfg}"
    );

    // Flipping back replaces (not appends) the keyword.
    g2.set_preferred_workflow("now").unwrap();
    assert_eq!(Graph::open(&root).meta().preferred_workflow, "now");
    let cfg = std::fs::read_to_string(root.join("logseq").join("config.edn")).unwrap();
    assert_eq!(
        cfg.matches(":preferred-workflow :now").count(),
        1,
        "no duplicate key: {cfg}"
    );

    // Inserting into a config that lacks the key entirely.
    std::fs::write(
        root.join("logseq").join("config.edn"),
        "{:start-of-week 0}\n",
    )
    .unwrap();
    let g3 = Graph::open(&root);
    g3.set_preferred_workflow("todo").unwrap();
    assert_eq!(Graph::open(&root).meta().preferred_workflow, "todo");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_default_journal_template_round_trips_in_config_edn() {
    let root = std::env::temp_dir().join(format!("tine-jtmpl-{}", std::process::id()));
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    // Existing config: a sibling key + a *commented* decoy that must NOT be edited.
    std::fs::write(
        &cfg_path,
        "{;; :default-templates {:journals \"Commented\"}\n :start-of-week 1}\n",
    )
    .unwrap();
    let cfg = || std::fs::read_to_string(&cfg_path).unwrap();
    let jtmpl = || Graph::open(&root).meta().default_journal_template;

    let g = Graph::open(&root);
    assert_eq!(jtmpl(), None, "unset to begin with");

    // Set (key absent → inserted, NOT touching the commented decoy).
    g.set_default_journal_template(Some("Daily")).unwrap();
    assert_eq!(jtmpl().as_deref(), Some("Daily"));
    assert!(
        cfg().contains(":start-of-week 1"),
        "sibling key preserved: {}",
        cfg()
    );
    assert!(
        cfg().contains(";; :default-templates {:journals \"Commented\"}"),
        "comment preserved: {}",
        cfg()
    );

    // Replace (no duplicate key).
    Graph::open(&root)
        .set_default_journal_template(Some("Weekly"))
        .unwrap();
    assert_eq!(jtmpl().as_deref(), Some("Weekly"));
    assert_eq!(
        cfg().matches(":journals").count(),
        2,
        "one real + one commented :journals: {}",
        cfg()
    );

    // A multi-word name round-trips.
    Graph::open(&root)
        .set_default_journal_template(Some("My Daily Log"))
        .unwrap();
    assert_eq!(jtmpl().as_deref(), Some("My Daily Log"));

    // Clear → back to factory default (blank journals).
    Graph::open(&root)
        .set_default_journal_template(None)
        .unwrap();
    assert_eq!(jtmpl(), None);
    assert!(
        cfg().contains(":start-of-week 1"),
        "sibling still preserved after clear: {}",
        cfg()
    );

    // Preserve a SIBLING key inside :default-templates when clearing :journals.
    std::fs::write(
        &cfg_path,
        "{:default-templates {:journals \"D\" :pages \"P\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(None)
        .unwrap();
    assert_eq!(jtmpl(), None);
    assert!(
        cfg().contains(":pages \"P\""),
        "sibling inner key preserved: {}",
        cfg()
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_default_journal_template_ignores_commented_journals_inside_map() {
    // `:journals` appears only inside a comment within the :default-templates map;
    // the writer must NOT edit the comment — it must insert a real key and leave
    // the comment + sibling untouched.
    let root = std::env::temp_dir().join(format!("tine-jtmpl-c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    std::fs::write(
        &cfg_path,
        "{:default-templates { ;; :journals \"Commented\"\n  :pages \"P\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(Some("Real"))
        .unwrap();
    let c = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(
        c.contains(":journals \"Real\""),
        "real journals inserted: {}",
        c
    );
    assert!(
        c.contains(";; :journals \"Commented\""),
        "comment untouched: {}",
        c
    );
    assert!(c.contains(":pages \"P\""), "sibling preserved: {}", c);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_default_journal_template_quoted_name_does_not_corrupt_config() {
    // A template name with an embedded quote is escaped on write; the escape-aware
    // value-end scan must then replace/clear only the value on the next edit,
    // never mis-scanning the `\"` and corrupting config.edn.
    let root = std::env::temp_dir().join(format!("tine-jtmpl-q-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    std::fs::write(
        &cfg_path,
        "{:start-of-week 1\n :default-templates {:journals \"Old\"}}\n",
    )
    .unwrap();
    let cfg = || std::fs::read_to_string(&cfg_path).unwrap();

    Graph::open(&root)
        .set_default_journal_template(Some("My \"Daily\""))
        .unwrap();
    assert!(
        cfg().contains("\\\""),
        "embedded quote should be escaped: {}",
        cfg()
    );

    // Replace with a plain name: clean single :journals, sibling intact, old value gone.
    Graph::open(&root)
        .set_default_journal_template(Some("Plain"))
        .unwrap();
    let c = cfg();
    assert!(
        c.contains(":journals \"Plain\""),
        "value replaced cleanly: {}",
        c
    );
    assert_eq!(
        c.matches(":journals").count(),
        1,
        "exactly one :journals: {}",
        c
    );
    assert!(c.contains(":start-of-week 1"), "sibling preserved: {}", c);
    assert!(
        !c.contains("Daily"),
        "old quoted value fully removed: {}",
        c
    );

    // Clearing a value that contains an escaped quote removes the whole pair.
    std::fs::write(&cfg_path, "{:default-templates {:journals \"a\\\"b\"}}\n").unwrap();
    Graph::open(&root)
        .set_default_journal_template(None)
        .unwrap();
    assert!(
        !cfg().contains(":journals"),
        "journals cleared, no garbage: {}",
        cfg()
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn set_default_journal_template_edn_aware_value_location() {
    // Round-4 audit: the value after `:journals` must be located/replaced as the
    // IMMEDIATE token, never "the next quote anywhere in the map" (which could land
    // on a later key's string value), and the outer `:default-templates` must not be
    // matched inside a string literal.
    let root = std::env::temp_dir().join(format!("tine-jtmpl-edn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");
    let cfg = || std::fs::read_to_string(&cfg_path).unwrap();

    // (a) `:journals` has a NON-string value (nil) and a later sibling has a string.
    // Setting must replace `nil`, not the `:pages` value.
    std::fs::write(
        &cfg_path,
        "{:default-templates {:journals nil :pages \"P\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(Some("X"))
        .unwrap();
    let c = cfg();
    assert!(
        c.contains(":journals \"X\""),
        "nil value replaced with X: {}",
        c
    );
    assert!(
        c.contains(":pages \"P\""),
        "later sibling string untouched: {}",
        c
    );
    assert!(!c.contains("nil"), "old nil value gone: {}", c);
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("X")
    );

    // Clearing the same shape removes only `:journals nil`, keeps `:pages "P"`.
    std::fs::write(
        &cfg_path,
        "{:default-templates {:journals nil :pages \"P\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(None)
        .unwrap();
    let c = cfg();
    assert!(!c.contains(":journals"), "journals pair removed: {}", c);
    assert!(
        c.contains(":pages \"P\""),
        "sibling string preserved on clear: {}",
        c
    );

    // (b) `:default-templates {…}` appears only INSIDE a string literal — it is not
    // the real key, so the writer must not edit it; it inserts a real key instead.
    std::fs::write(
        &cfg_path,
        "{:note \":default-templates {:journals \\\"fake\\\"}\"}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(Some("Real"))
        .unwrap();
    let c = cfg();
    assert!(
        c.contains(":journals \"Real\""),
        "real journals inserted: {}",
        c
    );
    assert!(
        c.contains("fake"),
        "string-literal decoy preserved verbatim: {}",
        c
    );
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("Real")
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn config_writers_skip_comment_between_key_and_value() {
    // A `;` comment between a key and its value must not mislead a writer (the
    // readers already skip it). Verify the writers do too — no duplicate key, the
    // real value is replaced.
    let root = std::env::temp_dir().join(format!("tine-wr-cmt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let cfg_path = root.join("logseq").join("config.edn");

    std::fs::write(&cfg_path, "{:favorites ; note\n [\"Old\"]}\n").unwrap();
    Graph::open(&root).set_favorites(&["New".into()]).unwrap();
    let c = std::fs::read_to_string(&cfg_path).unwrap();
    assert_eq!(
        c.matches(":favorites").count(),
        1,
        "replaced, not duplicated: {c}"
    );
    assert!(!c.contains("\"Old\""), "old value gone: {c}");
    assert_eq!(Graph::open(&root).meta().favorites, vec!["New".to_string()]);

    std::fs::write(
        &cfg_path,
        "{:default-templates {:journals ; note\n \"Old\"}}\n",
    )
    .unwrap();
    Graph::open(&root)
        .set_default_journal_template(Some("New"))
        .unwrap();
    assert_eq!(
        Graph::open(&root)
            .meta()
            .default_journal_template
            .as_deref(),
        Some("New")
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn deleted_journal_is_not_served_from_stale_cache() {
    use tine_core::PageKind;
    let root = std::env::temp_dir().join(format!("tine-del-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let jpath = root.join("journals").join("2026_06_18.md");
    std::fs::write(&jpath, "- hello\n").unwrap();

    let g = Graph::open(&root);
    // Warm the whole-graph cache so the journal is held in memory.
    let entries = g.journals_desc();
    assert_eq!(entries.len(), 1);
    let entry = entries[0].clone();
    assert!(g.load_page(&entry).is_ok());

    // External delete (OG Logseq / Syncthing) before the watcher reconciles.
    std::fs::remove_file(&jpath).unwrap();

    // load_page must report NotFound, NOT serve the cached copy — serving it with
    // a null rev would let a subsequent save recreate the deleted file.
    let err = g.load_page(&entry).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    // load_named treats the vanished page as absent (Ok(None)), not an error.
    assert!(g
        .load_named(&entry.name, PageKind::Journal)
        .unwrap()
        .is_none());
    // The stale entry was evicted, so the feed no longer lists it.
    assert!(
        g.journals_desc().is_empty(),
        "deleted journal must drop out of the feed"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn query_open_tasks() {
    let g = demo_graph();
    let groups = g.run_query("(task TODO DOING)");
    let raws: Vec<String> = groups
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(
        raws.iter().any(|r| r.starts_with("TODO Ship the M0")),
        "got: {raws:?}"
    );
    assert!(
        raws.iter().any(|r| r.starts_with("DOING Wire up")),
        "got: {raws:?}"
    );
    // A DONE task must not match.
    assert!(
        !raws.iter().any(|r| r.contains("DONE Validate")),
        "got: {raws:?}"
    );
}

#[test]
fn agenda_query_excludes_finished_tasks() {
    // The journal "Scheduled & Deadline" agenda (ui.ts::agendaQuery) must hide
    // DONE/CANCELED/CANCELLED items — matching OG's get-date-scheduled-or-deadlines
    // — while keeping open tasks AND marker-less scheduled blocks. A ±100y window
    // keeps the test robust against the real-clock `today` the engine resolves.
    let root = std::env::temp_dir().join(format!("tine-agenda-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_26.md"),
        "- TODO open task\n  SCHEDULED: <2026-06-27 Sat>\n\
         - DONE finished task\n  SCHEDULED: <2026-06-27 Sat>\n\
         - CANCELED dropped task\n  DEADLINE: <2026-06-27 Sat>\n\
         - CANCELLED british drop\n  DEADLINE: <2026-06-27 Sat>\n\
         - plain meeting\n  SCHEDULED: <2026-06-27 Sat>\n",
    )
    .unwrap();
    let g = Graph::open(&root);
    g.warm_cache();
    let q = "(and (or (between scheduled -36500d +36500d) (between deadline -36500d +36500d)) \
             (not (task DONE CANCELED CANCELLED)))";
    let raws: Vec<String> = g
        .run_query(q)
        .iter()
        .flat_map(|gr| gr.blocks.iter().map(|b| b.raw.clone()))
        .collect();
    assert!(
        raws.iter().any(|r| r.starts_with("TODO open task")),
        "open task missing: {raws:?}"
    );
    assert!(
        raws.iter().any(|r| r.starts_with("plain meeting")),
        "marker-less missing: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("DONE finished")),
        "DONE leaked: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("CANCELED dropped")),
        "CANCELED leaked: {raws:?}"
    );
    assert!(
        !raws.iter().any(|r| r.contains("CANCELLED british")),
        "CANCELLED leaked: {raws:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_superstring_rewrites_journal_and_nonjournal_refs() {
    // Regression for the reported case: the new name CONTAINS the old name, and
    // the old name is referenced from BOTH a non-journal page and a journal, with
    // the cache already warm + a backlinks query run first (as in live use).
    let root = std::env::temp_dir().join(format!("tine-rename-ss-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Testtest.md"), "- the page\n").unwrap();
    std::fs::write(
        root.join("pages").join("MyPage.md"),
        "- see [[Testtest]] here\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_15.md"),
        "- ref [[Testtest]]\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    g.warm_cache();
    let _ = g.backlinks("Testtest"); // populate the derived cache, as the UI does

    g.rename_page("Testtest", "TesttestTest").unwrap();

    let my = std::fs::read_to_string(root.join("pages").join("MyPage.md")).unwrap();
    let jr = std::fs::read_to_string(root.join("journals").join("2026_06_15.md")).unwrap();
    assert!(
        my.contains("[[TesttestTest]]") && !my.contains("[[Testtest]]"),
        "non-journal: {my}"
    );
    assert!(jr.contains("[[TesttestTest]]"), "journal: {jr}");
    let bl = g.backlinks("TesttestTest");
    let after: Vec<&str> = bl.iter().map(|x| x.page.as_str()).collect();
    assert!(
        after.contains(&"MyPage"),
        "backlinks miss non-journal: {after:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn rename_rewrites_nested_ref_in_open_page() {
    use tine_core::PageKind;
    // Mirror the reported "Tine" page: a NESTED ref (sub-bullet) with a block
    // id::, the page already LOADED (open/pinned), cache warm + backlinks queried.
    let root = std::env::temp_dir().join(format!("tine-rename-nested-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages").join("Testtest.md"), "- the page\n").unwrap();
    std::fs::write(
        root.join("pages").join("Tine.md"),
        "- Tine notes\n\t- see [[Testtest]] in a sub-bullet\n\t  id:: 1111aaaa-bbbb-cccc-dddd-eeeeeeeeeeee\n",
    )
    .unwrap();
    std::fs::write(
        root.join("journals").join("2026_06_15.md"),
        "- ref [[Testtest]]\n",
    )
    .unwrap();

    let g = Graph::open(&root);
    g.warm_cache();
    let _ = g.load_page(&g.find_entry("Tine", PageKind::Page).unwrap()); // simulate it being open
    let _ = g.backlinks("Testtest");

    g.rename_page("Testtest", "TesttestTest").unwrap();

    let tine = std::fs::read_to_string(root.join("pages").join("Tine.md")).unwrap();
    assert!(
        tine.contains("[[TesttestTest]]"),
        "nested ref NOT rewritten: {tine:?}"
    );
    assert!(!tine.contains("[[Testtest]]"), "old ref remains: {tine:?}");
    let bl = g.backlinks("TesttestTest");
    let pages: Vec<&str> = bl.iter().map(|x| x.page.as_str()).collect();
    assert!(pages.contains(&"Tine"), "backlinks miss Tine: {pages:?}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn trash_sync_conflict_refuses_real_pages() {
    let root = std::env::temp_dir().join(format!("tine-trashconflict-{}", std::process::id()));
    let pages = root.join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(pages.join("Real.md"), "- keep me\n").unwrap();
    let conflict = "Real.sync-conflict-20260705-120000-ABCDEFG.md";
    std::fs::write(pages.join(conflict), "- other device\n").unwrap();

    let g = Graph::open(&root);
    // Refuses a genuine page — never trashes real data.
    assert!(g.trash_sync_conflict("pages/Real.md").is_err());
    assert!(pages.join("Real.md").exists(), "real page must survive");
    // Trashes an actual conflict copy.
    g.trash_sync_conflict(&format!("pages/{conflict}")).unwrap();
    assert!(
        !pages.join(conflict).exists(),
        "conflict copy should be gone"
    );
    assert!(g.list_sync_conflicts().is_empty());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn sync_conflict_base_recognises_syncthing_and_dropbox() {
    use tine_core::model::sync_conflict_base;
    // Syncthing.
    assert_eq!(
        sync_conflict_base("Foo.sync-conflict-20260705-120000-ABCDEFG"),
        Some("Foo")
    );
    // Dropbox variants.
    assert_eq!(
        sync_conflict_base("Foo (conflicted copy 2026-07-05)"),
        Some("Foo")
    );
    assert_eq!(
        sync_conflict_base("Foo (martin's conflicted copy 2026-07-05)"),
        Some("Foo")
    );
    // Not a conflict copy.
    assert_eq!(sync_conflict_base("Foo"), None);
    assert_eq!(sync_conflict_base("2026_06_26"), None);
    assert_eq!(sync_conflict_base("My (draft) page"), None);
}

#[test]
fn sync_conflict_copies_excluded_from_pages_and_surfaced_separately() {
    let root = std::env::temp_dir().join(format!("tine-syncconflict-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    // A real page + its Syncthing conflict copy (shares the id::, so exercises the
    // "must not churn the id space" reason to keep it out of the cache).
    std::fs::write(
        root.join("pages").join("Foo.md"),
        "- hello\n  id:: aaaaaaaa-0000-0000-0000-0000000000ff\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages")
            .join("Foo.sync-conflict-20260705-120000-ABCDEFG.md"),
        "- hello from the other device\n  id:: aaaaaaaa-0000-0000-0000-0000000000ff\n",
    )
    .unwrap();
    // A journal + its conflict copy.
    std::fs::write(root.join("journals").join("2026_06_26.md"), "- day one\n").unwrap();
    std::fs::write(
        root.join("journals")
            .join("2026_06_26.sync-conflict-20260705-130000-ABCDEFG.md"),
        "- day one, edited elsewhere\n",
    )
    .unwrap();

    let g = Graph::open(&root);

    // The conflict copies must NOT appear as pages/journals.
    let names: Vec<String> = g.list_pages().into_iter().map(|p| p.name).collect();
    assert!(
        names.iter().any(|n| n == "Foo"),
        "real page missing: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.contains("sync-conflict")),
        "conflict copy leaked into page list: {names:?}"
    );

    // They ARE surfaced by list_sync_conflicts, each pointing at its winner.
    let mut conflicts = g.list_sync_conflicts();
    conflicts.sort_by(|a, b| a.base_name.cmp(&b.base_name));
    assert_eq!(conflicts.len(), 2, "conflicts: {conflicts:?}");
    let foo = conflicts
        .iter()
        .find(|c| c.base_name == "Foo")
        .expect("Foo conflict");
    assert_eq!(foo.base_path.as_deref(), Some("pages/Foo.md"));
    assert!(foo.tag.starts_with("sync-conflict-"), "tag: {}", foo.tag);
    assert!(
        foo.preview.contains("other device"),
        "preview: {}",
        foo.preview
    );
    let jrnl = conflicts
        .iter()
        .find(|c| c.base_name != "Foo")
        .expect("journal conflict");
    assert_eq!(jrnl.base_path.as_deref(), Some("journals/2026_06_26.md"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn resolve_sync_conflict_merges_and_trashes() {
    use std::collections::HashMap;
    let root = std::env::temp_dir().join(format!("tine-resolveconflict-{}", std::process::id()));
    std::fs::create_dir_all(root.join("journals")).unwrap();
    let pages = root.join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    let winner = "- alpha\n- beta line here\n  id:: aaaaaaaa-0000-0000-0000-0000000000b0\n";
    std::fs::write(pages.join("Foo.md"), winner).unwrap();
    let conflict = "- alpha\n- beta line there\n  id:: aaaaaaaa-0000-0000-0000-0000000000b0\n- extra from other device\n";
    let conflict_name = "Foo.sync-conflict-20260705-120000-ABCDEFG.md";
    std::fs::write(pages.join(conflict_name), conflict).unwrap();

    let g = Graph::open(&root);
    let win_rel = "pages/Foo.md";
    let conf_rel = format!("pages/{conflict_name}");

    // Diff to discover the row ids.
    let diff = g
        .sync_conflict_diff(win_rel, &conf_rel)
        .unwrap()
        .expect("a diff");
    let modified = diff
        .rows
        .iter()
        .find(|r| format!("{:?}", r.kind) == "Modified")
        .expect("modified row");
    let removed = diff
        .rows
        .iter()
        .find(|r| format!("{:?}", r.kind) == "Removed")
        .expect("removed row");

    assert_eq!(diff.base_rev, tine_core::model::content_rev(winner));
    // Guard: decisions from the diff must not apply after the winner changes.
    let changed_winner = winner.replace("beta line here", "beta line NEW!");
    std::fs::write(pages.join("Foo.md"), &changed_winner).unwrap();
    let err = g
        .resolve_sync_conflict(
            win_rel,
            &conf_rel,
            &HashMap::new(),
            &diff.base_rev,
            &diff.conflict_rev,
            "union",
        )
        .unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::AlreadyExists,
        "stale base_rev should conflict"
    );
    assert_eq!(
        std::fs::read_to_string(pages.join("Foo.md")).unwrap(),
        changed_winner,
        "winner untouched on guard"
    );
    std::fs::write(pages.join("Foo.md"), winner).unwrap();

    // The conflict side is revision-bound too. Otherwise decisions aligned to
    // an old copy could silently merge after a sync tool rewrites that copy.
    let changed_conflict = conflict.replace("beta line there", "beta line LATER");
    std::fs::write(pages.join(conflict_name), &changed_conflict).unwrap();
    let err = g
        .resolve_sync_conflict(
            win_rel,
            &conf_rel,
            &HashMap::new(),
            &diff.base_rev,
            &diff.conflict_rev,
            "union",
        )
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read_to_string(pages.join(conflict_name)).unwrap(),
        changed_conflict
    );
    assert_eq!(
        std::fs::read_to_string(pages.join("Foo.md")).unwrap(),
        winner
    );
    std::fs::write(pages.join(conflict_name), conflict).unwrap();

    // Resolve: take theirs for the modified block, pull in the removed one.
    let decisions = HashMap::from([
        (modified.id.clone(), "theirs".to_string()),
        (removed.id.clone(), "theirs".to_string()),
    ]);
    let base = tine_core::model::content_rev(winner);
    let conflict_rev = tine_core::model::content_rev(conflict);
    g.resolve_sync_conflict(
        win_rel,
        &conf_rel,
        &decisions,
        &base,
        &conflict_rev,
        "union",
    )
    .unwrap();

    let merged = std::fs::read_to_string(pages.join("Foo.md")).unwrap();
    assert!(merged.contains("beta line there"), "merged: {merged:?}");
    assert!(
        !merged.contains("beta line here"),
        "old winner text remains: {merged:?}"
    );
    assert!(
        merged.contains("extra from other device"),
        "removed block not pulled in: {merged:?}"
    );

    // Conflict copy is gone from pages/ and from the conflicts list, and lives in trash.
    assert!(
        !pages.join(conflict_name).exists(),
        "conflict copy not moved"
    );
    assert!(g.list_sync_conflicts().is_empty(), "conflict still listed");
    let trash = root.join("logseq").join(".tine-trash").join("conflicts");
    let trashed: Vec<_> = std::fs::read_dir(&trash).unwrap().flatten().collect();
    assert!(
        trashed
            .iter()
            .any(|e| e.file_name().to_string_lossy().contains("sync-conflict")),
        "conflict copy not in trash: {trashed:?}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(unix)]
#[test]
fn page_symlinks_are_not_indexed_or_reconciled() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("tine-page-symlink-{}", std::process::id()));
    let outside = std::env::temp_dir().join(format!("tine-outside-page-{}.md", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&outside);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(&outside, "- outside secret\n").unwrap();
    let link = root.join("pages/Secret.md");
    symlink(&outside, &link).unwrap();

    let g = Graph::open(&root);
    assert!(g.list_pages().iter().all(|page| page.name != "Secret"));
    assert!(g.resolve_rel("pages/Secret.md").is_none());
    assert!(g.sync_file(&link).is_none());

    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_file(&outside).ok();
}

#[cfg(unix)]
#[test]
fn checked_direct_graph_open_ignores_the_separate_managed_sync_namespace() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("tine-sync-link-{}", uuid::Uuid::new_v4()));
    let outside =
        std::env::temp_dir().join(format!("tine-sync-link-outside-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, root.join(".tine-sync")).unwrap();

    // Direct Files neither trusts nor owns `.tine-sync`; managed activation and
    // join validate it at their explicit boundary. A stale/broken managed
    // namespace must therefore not make an otherwise healthy Direct graph
    // unavailable.
    assert!(Graph::open_checked(&root).is_ok());

    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&outside).ok();
}

/// Editing one block must not rewrite bytes belonging to blocks the user never
/// touched. Direct Files data-safety audit, 2026-08-09.
///
/// The retention machinery in `doc.rs` existed but every production call site
/// passed an empty identity slice, so it was inert on the ordinary save path.
/// Measured consequence on a real-shaped 1,045-file graph: editing one block and
/// reverting it failed to restore the bytes of 96 of 983 files (9.8%).
mod untouched_bytes_survive_an_edit {
    use super::*;

    /// A scratch graph root; `tine-core`'s test deps do not include `tempfile`.
    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tine-untouched-bytes-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        root
    }

    fn edit_last_root(root: &std::path::Path, rel: &str, replacement: &str) -> String {
        let graph = Graph::open(root);
        graph.warm_cache();
        let mut page = graph.load_by_path(rel).unwrap().unwrap();
        let last = page.blocks.len() - 1;
        page.blocks[last].raw = replacement.into();
        graph.save_page(&page, page.rev.as_deref()).unwrap();
        std::fs::read_to_string(root.join(rel)).unwrap()
    }

    /// The dominant shape: 88 of the 96 damaged files were a page-level
    /// unbulleted `## Heading` that acquired a `- ` prefix.
    #[test]
    fn an_unbulleted_heading_keeps_its_missing_bullet() {
        let root = scratch("unbulleted-heading");
        let original = "- first bullet\n## Standalone heading\n- last bullet\n";
        std::fs::write(root.join("pages/Note.md"), original).unwrap();

        let after = edit_last_root(&root, "pages/Note.md", "last bullet edited");

        assert_eq!(
            after,
            original.replace("- last bullet\n", "- last bullet edited\n"),
            "editing the last block rewrote the untouched heading"
        );
    }

    /// A heading living in a bullet's continuation body used to have the source
    /// layout whitespace baked into its text (`\t  ## Section`) and gain a
    /// nesting level. Both are gone; assert the text and the outline shape, not
    /// the exact bytes — a whitespace-only separator line is still normalised to
    /// an empty line, which is recorded as a follow-up.
    #[test]
    fn a_heading_inside_a_continuation_body_keeps_its_text_and_depth() {
        let root = scratch("continuation-heading");
        let original =
            "- intro\n\t- body line one\n\t  \n\t  ## Section\n\t  \n\t  more prose\n- tail\n";
        std::fs::write(root.join("pages/Note.md"), original).unwrap();

        let after = edit_last_root(&root, "pages/Note.md", "tail edited");

        assert!(
            !after.contains("- \t  ##"),
            "source layout whitespace was injected into the block text: {after:?}"
        );
        assert!(
            after.contains("\t  ## Section"),
            "the heading lost its original indentation: {after:?}"
        );
        assert_eq!(
            after.matches("## Section").count(),
            1,
            "the heading was duplicated: {after:?}"
        );
        assert!(
            !after.contains("\t\t- "),
            "the block gained a nesting level: {after:?}"
        );
    }

    /// Necessity guard the other way: a save that changes nothing must still be
    /// a byte-exact no-op. Retention must not start inventing layout.
    #[test]
    fn an_unchanged_save_is_still_byte_exact() {
        let root = scratch("unchanged-save");
        let original = "- first bullet\n## Standalone heading\n- last bullet\n";
        std::fs::write(root.join("pages/Note.md"), original).unwrap();

        let graph = Graph::open(&root);
        graph.warm_cache();
        let page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
        graph.save_page(&page, page.rev.as_deref()).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("pages/Note.md")).unwrap(),
            original
        );
    }
}

/// A content-only save must not throw away the physical page inventory.
/// Direct Files perf audit, 2026-08-09, F1.
///
/// `list_pages` is memoized on `cache_gen`, and its only rebuild path re-reads
/// and re-parses every file in the graph (~35 µs/file, linear to 8,006 pages).
/// Every save bumped the generation unconditionally, so the first navigation or
/// `[[` autocomplete after any typing pause paid a whole-graph disk walk —
/// 243 ms on a real 5,225-file graph.
mod page_inventory_survives_a_content_save {
    use super::*;
    use std::time::Instant;

    fn graph_with(pages: usize, tag: &str) -> (PathBuf, Graph) {
        let root = std::env::temp_dir().join(format!(
            "tine-inventory-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        for i in 0..pages {
            std::fs::write(
                root.join(format!("pages/Page {i}.md")),
                format!("- body of page {i}\n- second block\n"),
            )
            .unwrap();
        }
        let graph = Graph::open(&root);
        graph.warm_cache();
        (root, graph)
    }

    fn save_first_block(graph: &Graph, rel: &str, raw: &str) {
        let mut page = graph.load_by_path(rel).unwrap().unwrap();
        page.blocks[0].raw = raw.into();
        graph.save_page(&page, page.rev.as_deref()).unwrap();
    }

    /// Keeping the inventory warm must never keep it WRONG. Regression for a
    /// defect I shipped with the optimization itself: the re-tag stamped a
    /// cached list as current no matter how old it was, so a list captured
    /// before a page was created got republished as authoritative — and since
    /// `list_pages` is keyed on generation equality it never rebuilt, leaving a
    /// page that exists on disk permanently unfindable.
    #[test]
    fn a_page_created_after_the_inventory_was_cached_is_still_found() {
        let (root, graph) = graph_with(8, "created-after-cache");
        graph.list_pages(); // cache the inventory WITHOUT the page below

        std::fs::write(root.join("pages/Latecomer.md"), "- arrived late\n").unwrap();
        graph.sync_file(&root.join("pages/Latecomer.md"));
        // A content-only save on an unrelated page is what re-tags the cache.
        save_first_block(&graph, "pages/Page 0.md", "edited body");

        assert!(
            graph.list_pages().iter().any(|p| p.name == "Latecomer"),
            "a page created after the inventory was cached vanished from it"
        );
        assert!(
            graph
                .load_named("Latecomer", tine_core::model::PageKind::Page)
                .unwrap()
                .is_some(),
            "the page exists on disk but cannot be loaded by name"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The perf property, as a RATIO rather than an absolute bound: a shared box
    /// makes absolute timings flaky, but a rebuild is ~350x a memo hit, so the
    /// gap survives any load. The `title::` save is the control — it genuinely
    /// changes the inventory and MUST still pay for a rebuild.
    #[test]
    fn a_content_only_save_keeps_the_inventory_warm() {
        let (root, graph) = graph_with(400, "warm");
        graph.list_pages();

        save_first_block(&graph, "pages/Page 1.md", "edited body");
        let start = Instant::now();
        let after_content = graph.list_pages();
        let content_only = start.elapsed();

        save_first_block(&graph, "pages/Page 2.md", "title:: Renamed Page Two");
        let start = Instant::now();
        let after_title = graph.list_pages();
        let identity_change = start.elapsed();

        assert_eq!(after_content.len(), after_title.len());
        assert!(
            content_only * 5 < identity_change,
            "a content-only save still paid for a whole-graph rebuild \
             (content-only {content_only:?} vs identity-change {identity_change:?})"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Necessity guard: keeping the memo must not make it stale. A `title::`
    /// edit moves the page's identity and the inventory has to follow.
    #[test]
    fn a_title_property_edit_still_updates_the_inventory() {
        let (root, graph) = graph_with(20, "title");
        assert!(graph.list_pages().iter().any(|p| p.name == "Page 3"));

        save_first_block(&graph, "pages/Page 3.md", "title:: Totally Different");

        let names: Vec<_> = graph.list_pages().into_iter().map(|p| p.name).collect();
        assert!(
            names.iter().any(|n| n == "Totally Different"),
            "the renamed page never appeared in the inventory: {names:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Necessity guard: a NEW file must appear even though the save that
    /// preceded it was content-only.
    #[test]
    fn a_new_page_still_appears_in_the_inventory() {
        let (root, graph) = graph_with(20, "new");
        graph.list_pages();
        save_first_block(&graph, "pages/Page 4.md", "edited body");
        std::fs::write(root.join("pages/Brand New.md"), "- hello\n").unwrap();

        let graph = Graph::open(&root);
        graph.warm_cache();
        assert!(graph.list_pages().iter().any(|p| p.name == "Brand New"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// GH #254: an external writer that publishes by `rename()` must not make the
/// open page permanently unsaveable.
///
/// Syncthing, Dropbox, Verysync, Logseq OG, VS Code and Vim with
/// `backupcopy=no` all publish temp-then-rename, which changes the inode. The
/// ordinary save refused on that alone, BEFORE comparing bytes, with
/// `path-pinned page does not match its captured exact owner` — a code the
/// frontend classifies as transient and retries forever, so the page silently
/// stopped saving. Direct Files data-safety audit, 2026-08-09, finding 2.
///
/// Increment 2 adds one-shot exact-snapshot authority to the conflict half;
/// trusted journal projection remains deliberately separate.
mod external_atomic_replacement {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tine-gh254-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        root
    }

    /// Replace `pages/Note.md` the way a syncing tool does: write a temp file in
    /// the same directory, then rename it over the target. New inode.
    fn deliver(root: &std::path::Path, bytes: &str) {
        let tmp = root.join("pages/.delivery.tmp");
        std::fs::write(&tmp, bytes).unwrap();
        std::fs::rename(&tmp, root.join("pages/Note.md")).unwrap();
    }

    fn open_with(root: &std::path::Path, bytes: &str) -> (Graph, tine_core::model::PageDto) {
        std::fs::write(root.join("pages/Note.md"), bytes).unwrap();
        let graph = Graph::open(root);
        graph.warm_cache();
        let page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
        (graph, page)
    }

    /// The headline case. Identical bytes on a new inode are a republication of
    /// the state the editor already has, not a conflict — and no watcher event
    /// is needed for the save to work.
    #[test]
    fn a_same_byte_delivery_does_not_block_the_save() {
        let root = scratch("same-bytes");
        let (graph, mut page) = open_with(&root, "- original\n");
        let base = page.rev.clone();

        deliver(&root, "- original\n");

        page.blocks[0].raw = "mine".into();
        graph
            .save_page(&page, base.as_deref())
            .expect("a same-byte atomic replacement must not block the save");
        assert!(std::fs::read_to_string(root.join("pages/Note.md"))
            .unwrap()
            .contains("mine"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// And it must keep working: the old refusal was permanent for the loaded
    /// instance, which is what turned this into "my notes stopped saving".
    #[test]
    fn a_second_save_after_a_same_byte_delivery_also_works() {
        let root = scratch("same-bytes-twice");
        let (graph, mut page) = open_with(&root, "- original\n");
        let base = page.rev.clone();
        deliver(&root, "- original\n");

        page.blocks[0].raw = "mine".into();
        let rev = graph.save_page(&page, base.as_deref()).unwrap();
        page.blocks[0].raw = "mine again".into();
        graph
            .save_page(&page, Some(rev.as_str()))
            .expect("the page must not become permanently unsaveable");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Necessity guard, and the whole point of the byte comparison: DIFFERENT
    /// bytes are a real conflict and must still refuse — with the literal
    /// `conflict` code the frontend can actually resolve, not a physical-identity
    /// message it retries forever.
    #[test]
    fn a_different_byte_delivery_is_a_resolvable_conflict() {
        let root = scratch("diff-bytes");
        let (graph, mut page) = open_with(&root, "- original\n");
        let base = page.rev.clone();

        deliver(&root, "- from another device\n");

        page.blocks[0].raw = "mine".into();
        let error = graph.save_page(&page, base.as_deref()).unwrap_err();
        assert_eq!(
            tine_core::model::direct_save_failure_code(&error),
            "conflict.save_baseline_present",
            "must be a resolvable minted-authority code"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("pages/Note.md")).unwrap(),
            "- from another device\n",
            "the other device's bytes must survive"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The same conflict delivered in place (same inode) must behave identically
    /// — the two shapes were resolvable and unresolvable respectively, which is
    /// how the inode was identified as the discriminator.
    #[test]
    fn an_in_place_different_byte_change_conflicts_the_same_way() {
        let root = scratch("inplace-diff");
        let (graph, mut page) = open_with(&root, "- original\n");
        let base = page.rev.clone();

        std::fs::write(root.join("pages/Note.md"), "- edited in place\n").unwrap();

        page.blocks[0].raw = "mine".into();
        let error = graph.save_page(&page, base.as_deref()).unwrap_err();
        assert_eq!(
            tine_core::model::direct_save_failure_code(&error),
            "conflict.save_baseline_present"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An external deletion is a resolvable `Absent` conflict. It must not be
    /// resurrected until the user explicitly chooses Keep mine.
    #[test]
    fn an_external_delete_mints_absent_authority_without_resurrecting() {
        let root = scratch("deleted");
        let (graph, mut page) = open_with(&root, "- original\n");
        let base = page.rev.clone();
        as_editor(&graph, &mut page);

        std::fs::remove_file(root.join("pages/Note.md")).unwrap();

        page.blocks[0].raw = "mine".into();
        let error = graph.save_page(&page, base.as_deref()).unwrap_err();
        assert_eq!(
            tine_core::model::direct_save_failure_code(&error),
            "conflict.save_baseline_absent"
        );
        assert!(
            !root.join("pages/Note.md").exists(),
            "a deleted page must not be silently resurrected"
        );
        let shown = graph
            .outstanding_conflict_override(&page)
            .unwrap()
            .expect("the refused save names the conflict shown to this editor");
        graph
            .force_save_page_at_revision(&page, base.as_deref(), shown)
            .unwrap();
        assert!(std::fs::read_to_string(root.join("pages/Note.md"))
            .unwrap()
            .contains("mine"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// GH #270 at the boundary the reporter observed: the Unlinked References
/// panel of a page, fed by `Graph::unlinked_refs`.
mod unlinked_references_see_code_blocks {
    use super::*;

    fn graph_with(source: &str) -> (std::path::PathBuf, Graph) {
        let root = std::env::temp_dir().join(format!(
            "tine-gh270-{}-{}",
            std::process::id(),
            source.len()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/Widget.md"), "- the page itself\n").unwrap();
        std::fs::write(root.join("pages/Notes.md"), source).unwrap();
        let graph = Graph::open(&root);
        graph.warm_cache();
        (root, graph)
    }

    #[test]
    fn a_mention_inside_a_fenced_code_block_is_reported() {
        let (root, graph) = graph_with("- how to build it\n  ```sh\n  make Widget\n  ```\n");
        let groups = graph.unlinked_refs("Widget");
        assert_eq!(
            groups
                .iter()
                .map(|group| group.page.as_str())
                .collect::<Vec<_>>(),
            vec!["Notes"],
            "the fenced mention never reached the panel"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_mention_inside_inline_code_or_math_is_reported() {
        for source in [
            "- run `Widget --help` first\n",
            "- the model\n  $$\n  W = Widget(x)\n  $$\n",
        ] {
            let (root, graph) = graph_with(source);
            assert_eq!(graph.unlinked_refs("Widget").len(), 1, "{source}");
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// The complement is not "everything": an existing link is still a LINKED
    /// reference, and clock entries are stripped exactly as Logseq strips them.
    #[test]
    fn linked_syntax_and_logbook_clock_lines_stay_out() {
        let (root, graph) = graph_with("- see [[Widget]]\n  :LOGBOOK:\n  CLOCK: Widget\n  :END:\n");
        assert!(
            graph.unlinked_refs("Widget").is_empty(),
            "an explicit link or a logbook line leaked into unlinked references"
        );
        assert_eq!(
            graph.backlinks("Widget").len(),
            1,
            "the link is still linked"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// Direct Files data-safety audit, finding 15. Two shadow-journal classifiers
/// disagreed: the managed one asked every Logseq text extension, the direct one
/// hard-coded `.md`/`.org`, though `.markdown` is first-class
/// (`LOGSEQ_TEXT_EXTENSIONS`, and OG accepts it case-insensitively).
///
/// NOT a regression proof, and deliberately labelled as such: these three pass
/// with and without the unification. The audit predicted that a `.markdown`
/// canonical day would let a title-named leftover poison the (kind, name) cache,
/// and it does not — `load_named` and `journals_desc` both still serve the
/// canonical file, because later layers happen to mask the misclassification. So
/// the divergence is real in the code and its predicted consequence is not
/// reachable today. What these DO pin is the #21 rule itself, at the observation
/// boundary, for every extension: whichever layer is currently masking it can
/// move without silently taking the guarantee with it.
mod shadow_journal_sees_every_text_extension {
    use super::*;

    fn graph_with_canonical(extension: &str) -> (std::path::PathBuf, Graph) {
        let root = std::env::temp_dir().join(format!(
            "tine-ds15-shadow-{}-{extension}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        // The canonical day, under a date-stem filename.
        std::fs::write(
            root.join(format!("journals/2026_06_26.{extension}")),
            "- the canonical day\n",
        )
        .unwrap();
        // …and a leftover whose NAME parses as that same day. Nothing on disk
        // says which is authoritative, so the date-stem file wins by rule and
        // this one must stay out of the (kind, name) cache.
        std::fs::write(
            root.join("pages/leftover.md"),
            "title:: Jun 26th, 2026\n- the shadow's own text\n",
        )
        .unwrap();
        let graph = Graph::open(&root);
        graph.warm_cache();
        (root, graph)
    }

    fn day_resolves_to_the_canonical_file(extension: &str) {
        let (root, graph) = graph_with_canonical(extension);
        // Reading the leftover by PATH is the ordinary way it gets seen: opening
        // the stray file, or a watcher event on it. That read reconciles the
        // parsed document into the (kind, name) cache unless the shadow rule
        // stops it — which is where the two classifiers disagreed.
        let leftover = graph
            .list_pages()
            .into_iter()
            .find(|entry| entry.rel_path == "pages/leftover.md")
            .expect("the leftover is discovered");
        let _ = graph.load_page(&leftover);

        let loaded = graph
            .load_named("Jun 26th, 2026", tine_core::PageKind::Journal)
            .expect("load_named succeeds")
            .expect("the day resolves to something");
        let text = format!("{loaded:?}");
        assert!(
            text.contains("the canonical day"),
            ".{extension}: the day resolved to the shadow instead of the canonical file"
        );
        assert!(
            !text.contains("the shadow's own text"),
            ".{extension}: the shadow leaked into the day's content"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_md_canonical_day_shadows_a_title_named_leftover() {
        day_resolves_to_the_canonical_file("md");
    }

    #[test]
    fn a_markdown_canonical_day_shadows_it_too() {
        day_resolves_to_the_canonical_file("markdown");
    }

    #[test]
    fn an_org_canonical_day_shadows_it_too() {
        day_resolves_to_the_canonical_file("org");
    }
}
