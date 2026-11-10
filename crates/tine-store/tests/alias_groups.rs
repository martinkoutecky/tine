//! D-20261019-AL1/AL2 (queue card hB9Zjw): a page's alias group, split into
//! the names with no file of their own and the real pages, and the one owner
//! rule an alias-only name with several owners opens.

use std::fs;

use tine_store::{InventoryScope, PageId, Resolved, Store, WholeGraph};

fn graph(files: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::create_dir(dir.path().join("journals")).unwrap();
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    (dir, store)
}

/// `name`'s alias group as the page header splits it: (names with no file,
/// names a file claims). Every member's target is the one `resolve` gives.
fn group(view: &WholeGraph, name: &str) -> (Vec<String>, Vec<String>) {
    let (mut aliases, mut pages) = (Vec::new(), Vec::new());
    for entry in &view.inventory(InventoryScope::AliasGroup(name)).0 {
        assert_eq!(
            ids(&entry.target),
            ids(&view.resolve(&entry.name, entry.is_journal))
        );
        match entry.target {
            Resolved::Existing { .. } => pages.push(entry.name.clone()),
            Resolved::Alias { .. } => aliases.push(entry.name.clone()),
            Resolved::Absent { .. } => panic!("{} is in a group but names nothing", entry.name),
        }
    }
    (aliases, pages)
}

fn ids(target: &Resolved) -> Vec<PageId> {
    match target {
        Resolved::Existing { id, others } => [vec![id.clone()], others.clone()].concat(),
        Resolved::Alias { owners } => owners.clone(),
        Resolved::Absent { .. } => Vec::new(),
    }
}

fn names(aliases: &[&str], pages: &[&str]) -> (Vec<String>, Vec<String>) {
    let owned = |list: &[&str]| list.iter().map(|s| s.to_string()).collect();
    (owned(aliases), owned(pages))
}

#[test]
fn date_alias_group_never_acquires_the_ordinary_canonical_claimant() {
    let _guard = crate::integration_lock();
    let (dir, store) = graph(&[
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"yyyy_MM_dd\"}\n",
        ),
        (
            "pages/2026_10_23.md",
            "alias:: Literal\n\n- unrelated ordinary file\n",
        ),
        ("pages/Owner.md", "alias:: 2026-10-23\n\n- owner\n"),
    ]);
    let check = || {
        let view = store.whole_graph().unwrap();
        let entries = view.inventory(InventoryScope::AliasGroup("Owner"));
        assert_eq!(entries.0.len(), 1, "one day member: {entries:?}");
        let member = &entries.0[0];
        assert!(
            member.is_journal,
            "ordinary claimant is unrelated: {member:?}"
        );
        assert_eq!(member.day, Some(tine_store::Day(20261023)));
        assert!(matches!(member.target, Resolved::Alias { .. }));
        assert_eq!(view.link_target(&member.name).name, "Owner");
        assert_eq!(group(&view, "2026-10-23"), names(&[], &["Owner"]));
        assert_eq!(group(&view, "2026_10_23"), names(&["Literal"], &[]));
    };
    check();
    fs::write(
        dir.path().join("pages/Owner.md"),
        "alias:: 2026-10-23, [[Oct 23rd, 2026]]\n\n- owner\n",
    )
    .unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    check();
    fs::write(dir.path().join("journals/2026_10_23.md"), "- journal\n").unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    // The ordinary opening claimant masks this physical journal (OWN2).
    check();
    fs::remove_file(dir.path().join("pages/2026_10_23.md")).unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let view = store.whole_graph().unwrap();
    assert_eq!(group(&view, "Owner"), names(&[], &["2026_10_23"]));
    for name in ["2026_10_23", "2026-10-23"] {
        assert_eq!(group(&view, name), names(&[], &["Owner"]));
    }
    store.close();
}

#[test]
fn date_alias_endpoints_share_opening_and_reference_class_without_a_journal_file() {
    let _guard = crate::integration_lock();
    let (dir, store) = graph(&[
        ("pages/Owner.md", "alias:: [[Oct 23rd, 2026]]\n\n- owner\n"),
        (
            "pages/Notes.md",
            "- alternate [[2026-10-23]]\n- canonical [[Oct 23rd, 2026]]\n",
        ),
    ]);
    let view = store.whole_graph().unwrap();
    for name in ["Owner", "Oct 23rd, 2026", "2026_10_23", "2026-10-23"] {
        let target = view.link_target(name);
        assert_eq!(target.name, "Owner", "opening {name}");
        assert!(!target.is_journal);
        let rows = view.backlinks(name).unwrap();
        let blocks: usize = rows.iter().map(|row| row.blocks.len()).sum();
        assert_eq!(blocks, 2, "references {name}: {rows:?}");
    }
    assert_eq!(group(&view, "Owner"), names(&["Oct 23rd, 2026"], &[]));
    assert_eq!(group(&view, "2026-10-23"), names(&[], &["Owner"]));
    drop(view);
    let literal = dir.path().join("pages/2026-10-23.md");
    fs::write(&literal, "- literal ordinary date file\n").unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let view = store.whole_graph().unwrap();
    assert_eq!(view.link_target("2026-10-23").name, "2026-10-23");
    assert_eq!(view.link_target("2026_10_23").name, "Owner");
    assert_eq!(view.backlinks("Owner").unwrap()[0].blocks.len(), 1);
    drop(view);
    fs::remove_file(literal).unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    assert_eq!(
        store.whole_graph().unwrap().backlinks("Owner").unwrap()[0]
            .blocks
            .len(),
        2
    );
    store.close();
}

#[test]
fn configured_date_aliases_survive_a_warm_checkpoint() {
    let _guard = crate::integration_lock();
    let (dir, store) = graph(&[
        ("logseq/config.edn", "{:journal/file-name-format \"dd.MM.yyyy\" :journal/page-title-format \"yyyy_dd_MM\"}\n"),
        ("pages/Owner.md", "alias:: 2026_23_10\n\n- owner\n"),
        ("pages/Notes.md", "- [[23.10.2026]]\n- [[2026-10-23]]\n"),
    ]);
    store.close();
    let cp = dir.path().join("launch.bin");
    let open = || {
        Store::open(
            dir.path(),
            tine_store::OpenOptions {
                launch_checkpoint: Some(cp.clone()),
                watch: tine_store::WatchMode::Poll,
                ..Default::default()
            },
        )
        .unwrap()
        .0
    };
    for warm in [false, true] {
        let store = open();
        let view = store.whole_graph().unwrap();
        for name in ["Owner", "2026_23_10", "23.10.2026", "2026-10-23"] {
            assert_eq!(view.link_target(name).name, "Owner", "{name}, warm={warm}");
            assert_eq!(
                view.backlinks(name).unwrap()[0].blocks.len(),
                2,
                "{name}, warm={warm}"
            );
            let tine_store::QueryResult::Simple(groups) = view
                .query(&format!("[[{name}]]"), tine_store::QueryDialect::Simple)
                .unwrap()
            else {
                panic!("block query")
            };
            assert_eq!(
                groups
                    .iter()
                    .find(|group| group.page == "Notes")
                    .unwrap()
                    .blocks
                    .len(),
                2
            );
        }
        assert!(
            !view.inventory(InventoryScope::All).0.iter().any(|entry| {
                entry.day == Some(tine_store::Day(20261023))
                    && matches!(entry.target, Resolved::Absent { .. })
            }),
            "an alias-only day is not a second page entity"
        );
        if warm {
            assert_eq!(
                store.diagnostics()["checkpoint"]["load"]["outcome"],
                "loaded"
            );
        }
        assert!(store.write_checkpoint_now().is_some());
        store.close();
    }
}

#[test]
fn configured_date_aliases_keep_case_sensitive_literals_and_loose_digits() {
    let _guard = crate::integration_lock();
    let format = tine_core::date::JournalFormat::new(Some("dd.MM.yyyy"), Some("X-yyyy_d_MM"));
    assert!(
        format.parse("X-2026_03_10").is_some(),
        "recognized authored spelling"
    );
    let (dir, store) = graph(&[
        ("logseq/config.edn", "{:journal/file-name-format \"dd.MM.yyyy\" :journal/page-title-format \"X-yyyy_d_MM\"}\n"),
        ("pages/Owner.md", "alias:: X-2026_3_10\n\n- owner\n"),
        ("pages/Notes.md", "- [[X-2026_03_10]]\n- [[2026-10-03]]\n"),
    ]);
    let check = |aliased: bool, count: usize| {
        let view = store.whole_graph().unwrap();
        let inventory = view.inventory(InventoryScope::All);
        let day = inventory
            .0
            .iter()
            .find(|entry| entry.name == "X-2026_3_10")
            .expect("canonical day spelling");
        assert!(day.is_journal);
        assert_eq!(matches!(day.target, Resolved::Alias { .. }), aliased);
        for name in ["X-2026_3_10", "X-2026_03_10", "2026-10-03"] {
            assert_eq!(
                view.backlinks(name).unwrap()[0].blocks.len(),
                count,
                "{name}"
            );
            assert_eq!(
                view.link_target(name).name,
                if aliased { "Owner" } else { "X-2026_3_10" }
            );
            let tine_store::QueryResult::Simple(groups) = view
                .query(&format!("[[{name}]]"), tine_store::QueryDialect::Simple)
                .unwrap()
            else {
                panic!("reference query")
            };
            assert_eq!(
                groups
                    .iter()
                    .find(|group| group.page == "Notes")
                    .unwrap()
                    .blocks
                    .len(),
                count
            );
        }
    };
    check(true, 2);
    let notes = dir.path().join("pages/Notes.md");
    fs::write(&notes, "- [[X-2026_03_10]]\n").unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    check(true, 1);
    let changes = store.subscribe();
    fs::write(&notes, "- edited [[X-2026_03_10]]\n").unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let notified = std::iter::from_fn(|| changes.try_recv().unwrap()).any(|change| {
        serde_json::to_value(change).unwrap()["summary"]["linksTo"]
            .as_array()
            .is_some_and(|names| names.iter().any(|name| name == "owner"))
    });
    assert!(
        notified,
        "the authored date spelling invalidates the alias owner's references"
    );
    let tine_store::QueryResult::Simple(groups) = store
        .whole_graph()
        .unwrap()
        .query("[[Owner]]", tine_store::QueryDialect::Simple)
        .unwrap()
    else {
        panic!("reference query")
    };
    assert!(
        groups
            .iter()
            .find(|group| group.page == "Notes")
            .unwrap()
            .blocks[0]
            .raw
            .contains("edited"),
        "memo follows a loose date spelling edit"
    );
    fs::write(dir.path().join("pages/Owner.md"), "- owner\n").unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    check(false, 1);
    store.close();
}

/// A chain (Alpha → Beta → Gamma) with fileless names on two links is one
/// group: every real page lists the same fileless names, as declared, and the
/// other real pages, never itself; an alias-only name answers for its owner's
/// group.
#[test]
fn a_group_separates_fileless_names_from_real_pages() {
    let _guard = crate::integration_lock();
    let (_dir, store) = graph(&[
        ("pages/Alpha.md", "alias:: Al, Beta\n\n- alpha notes\n"),
        ("pages/Beta.md", "alias:: be, Gamma\n\n- beta notes\n"),
        ("pages/Gamma.md", "- gamma notes\n"),
        ("pages/Lonely.md", "- no aliases\n"),
    ]);
    let view = store.whole_graph().unwrap();
    assert_eq!(
        group(&view, "Alpha"),
        names(&["Al", "be"], &["Beta", "Gamma"])
    );
    assert_eq!(
        group(&view, "beta"),
        names(&["Al", "be"], &["Alpha", "Gamma"])
    );
    assert_eq!(
        group(&view, "Gamma"),
        names(&["Al", "be"], &["Alpha", "Beta"])
    );
    assert_eq!(
        group(&view, "AL"),
        names(&["be"], &["Alpha", "Beta", "Gamma"])
    );
    assert_eq!(group(&view, "Lonely"), names(&[], &[]));
}

/// A self-alias and a twice-declared name are one name each; a name declared
/// in two spellings shows the least one.
#[test]
fn self_aliases_and_repeated_declarations_add_no_member() {
    let _guard = crate::integration_lock();
    let (_dir, store) = graph(&[
        ("pages/Solo.md", "alias:: Solo, so\n\n- x\n"),
        ("pages/Other.md", "alias:: SO\n\n- y\n"),
    ]);
    let view = store.whole_graph().unwrap();
    assert_eq!(group(&view, "Solo"), names(&["SO"], &["Other"]));
    assert_eq!(group(&view, "Other"), names(&["SO"], &["Solo"]));
}

/// The owner rule: an alias-only name declared by several pages opens the
/// owner whose graph-relative file path sorts first, as plain text. The
/// backend resolver (page by name, search) and the inventory the frontend
/// navigates by give the same first owner, also when one owner lives in a
/// folder whose name sorts between the other's characters.
#[test]
fn every_resolver_picks_the_same_first_owner() {
    let _guard = crate::integration_lock();
    let (_dir, store) = graph(&[
        (
            "pages/a/x.md",
            "title:: Nested\nalias:: shared\n\n- nested\n",
        ),
        ("pages/a b.md", "alias:: shared\n\n- spaced\n"),
    ]);
    let view = store.whole_graph().unwrap();
    let Resolved::Alias { owners } = view.resolve("shared", false) else {
        panic!("shared is an alias");
    };
    let inventory = view.inventory(InventoryScope::All);
    let listed = inventory
        .0
        .iter()
        .find_map(|entry| match &entry.target {
            Resolved::Alias { owners } if entry.name == "shared" => Some(owners.clone()),
            _ => None,
        })
        .expect("the inventory lists the alias");
    assert_eq!(owners[0], PageId::from("pages/a b.md".to_owned()));
    assert_eq!(owners[0], listed[0]);
    let opened = store.page_named("shared", tine_core::model::PageKind::Page);
    assert_eq!(opened.unwrap().unwrap().id, owners[0]);
}
