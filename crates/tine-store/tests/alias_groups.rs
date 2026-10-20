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

/// A chain (Alpha → Beta → Gamma) with fileless names on two links is one
/// group: every real page lists the same fileless names, as declared, and the
/// other real pages, never itself; an alias-only name answers for its owner's
/// group.
#[test]
fn a_group_separates_fileless_names_from_real_pages() {
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
