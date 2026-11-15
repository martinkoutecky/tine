//! Conceptual model step 2 (batch 6): the stuck-page check of a rename
//! (GH #535, `document/graphRewrite.ts unsavedPathsFor`) asks the rename
//! itself, `pages::rename_would_change`, whether a page's unsaved text would
//! be rewritten. These are the reference cases `renameStuckPage.test.ts` used
//! to pin on a frontend walker; each is pinned here on the answer AND on what
//! the real rename then writes, so the two cannot disagree.
use std::fs;
use std::path::PathBuf;
use tine_graph_features::pages::{self, RenameProbeText};
use tine_store::Store;

fn fixture(files: &[(&str, &str)]) -> (PathBuf, std::sync::Arc<Store>) {
    let root = crate::test_support::fresh_dir("tine-rename-would-change-");
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (root, store)
}

fn block(raw: &str) -> RenameProbeText {
    RenameProbeText {
        raw: raw.into(),
        org: false,
        pre_block: false,
    }
}

#[test]
fn the_answer_is_what_the_rename_writes() {
    let _integration_guard = crate::integration_lock();
    // (label, renamed page, its file, the other page's one block, rewritten)
    let cases: &[(&str, &str, &str, &str, bool)] = &[
        (
            "page ref, other casing",
            "Old",
            "Old.md",
            "draft about [[old]]",
            true,
        ),
        (
            "decomposed reference to an NFC name",
            "Café",
            "Café.md",
            "see [[Cafe\u{301}]]",
            true,
        ),
        (
            "NFC reference to a decomposed name",
            "Cafe\u{301}",
            "Cafe\u{301}.md",
            "see [[Café]]",
            true,
        ),
        ("#tag", "Old", "Old.md", "a #Old tag", true),
        ("#[[tag]]", "Old", "Old.md", "a #[[old]] tag", true),
        (
            "bare tags:: value",
            "Old",
            "Old.md",
            "draft\ntags:: Other, Old",
            true,
        ),
        (
            "namespace child reference",
            "Old",
            "Old.md",
            "see [[Old/Child]]",
            true,
        ),
        (
            "reference in a property value",
            "Old",
            "Old.md",
            "draft\nrelated:: [[Old]]",
            true,
        ),
        (
            "reference in an embed",
            "Old",
            "Old.md",
            "{{embed [[Old]]}}",
            true,
        ),
        (
            "reference in a cloze argument",
            "Old",
            "Old.md",
            "{{cloze [[Old]]}}",
            true,
        ),
        (
            "a query's page name",
            "Old",
            "Old.md",
            "{{query [[Old]]}}",
            true,
        ),
        (
            "a property key, as OG",
            "Old",
            "Old.md",
            "draft\nold:: x",
            true,
        ),
        (
            "a non-ASCII property key",
            "Über",
            "Über.md",
            "draft\nÜber:: x",
            true,
        ),
        (
            "prose containing the name",
            "Cat",
            "Cat.md",
            "we educate cats",
            false,
        ),
        (
            "a longer page name",
            "Old",
            "Old.md",
            "see [[Older]] and #Oldest",
            false,
        ),
        (
            "a namespace parent, not child",
            "Old/Child",
            "Old___Child.md",
            "see [[Old]]",
            false,
        ),
        (
            "a reference inside inline code",
            "Old",
            "Old.md",
            "literal `[[Old]]` here",
            false,
        ),
    ];
    for (label, from, file, draft, expected) in cases {
        let page = format!("pages/{file}");
        let other = format!("- {draft}\n");
        let (root, store) = fixture(&[
            (page.as_str(), "- body\n"),
            ("pages/Old___Child.md", "- child\n"),
            ("pages/Other.md", other.as_str()),
        ]);
        let answer = pages::rename_would_change(&store, from, "New", &[block(draft)]).unwrap();
        assert_eq!(answer, vec![*expected], "{label}");
        {
            let host = crate::test_host(&store);
            pages::rename_or_merge_page(&store, &host, from, "New", None, None, &[]).unwrap();
        }
        let after = fs::read_to_string(root.join("pages/Other.md")).unwrap();
        assert_eq!(
            after != other,
            *expected,
            "{label}: the rename wrote {after:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

/// The planner reads a page whose only mention of the renamed page is a
/// rewrite target (no reference selects it): a link in a macro argument, in a
/// block or in the pre-block, and an Org file link whose label names another
/// page. Each file is rewritten on disk.
#[test]
fn a_page_holding_only_rewrite_targets_is_rewritten() {
    let _integration_guard = crate::integration_lock();
    let files: &[(&str, &str)] = &[
        ("pages/Old.org", "* body\n"),
        ("pages/R.md", "- {{renderer :x, [[Old]]}}\n"),
        ("pages/P.md", "note:: {{cloze [[Old]]}}\n\n- body\n"),
        ("pages/F.org", "* see [[file:./Old.org][another name]]\n"),
        // The name spelled with an escape: the planner's substring prefilter
        // must not skip these.
        ("pages/It's.md", "- body\n"),
        ("pages/T.md", "- {{tine-query @page name = 'IT''S'}}\n"),
        ("pages/Q.md", "- {{query (page \"Ol\\\\d\")}}\n"),
        ("pages/Ol.md", "title:: Ol\\d\n\n- body\n"),
    ];
    let (root, store) = fixture(files);
    {
        let host = crate::test_host(&store);
        pages::rename_or_merge_page(&store, &host, "Old", "New", None, None, &[]).unwrap();
        pages::rename_or_merge_page(&store, &host, "It's", "Its", None, None, &[]).unwrap();
        pages::rename_or_merge_page(&store, &host, "Ol\\d", "Olx", None, None, &[]).unwrap();
    }
    let read = |rel: &str| fs::read_to_string(root.join(rel)).unwrap();
    for (rel, before) in &files[1..4] {
        let after = read(rel);
        assert_ne!(&after, before, "{rel} was not rewritten");
        assert!(after.contains("New"), "{rel}: {after:?}");
    }
    assert_eq!(read("pages/T.md"), "- {{tine-query @page name = 'Its'}}\n");
    assert_eq!(read("pages/Q.md"), "- {{query (page \"Olx\")}}\n");
    let _ = fs::remove_dir_all(&root);
}

/// The typed namespace map covers unclaimed descendants too (CM12/CM13),
/// so a held draft must block the rename when it would change that reference.
#[test]
fn a_descendant_no_file_owns_is_rewritten() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = fixture(&[("pages/Old.md", "- body\n")]);
    let answer = pages::rename_would_change(
        &store,
        "Old",
        "New",
        &[block("see [[Old/Child]]"), block("see [[Old]]")],
    )
    .unwrap();
    assert_eq!(answer, vec![true, true]);
    let _ = fs::remove_dir_all(&root);
}

/// CM13: held bare alias values change just as explicit links do.
#[test]
fn bare_alias_members_follow() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = fixture(&[("pages/Alias.md", "- body\n")]);
    let answer = pages::rename_would_change(
        &store,
        "Alias",
        "New",
        &[block("x\nalias:: Alias"), block("x\nalias:: [[Alias]]")],
    )
    .unwrap();
    assert_eq!(answer, vec![true, true]);
    let _ = fs::remove_dir_all(&root);
}

/// A page's pre-block is asked as a pre-block (its page properties), each
/// text separately, and Org as Org.
#[test]
fn pre_blocks_and_org_texts_are_read_as_such() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = fixture(&[("pages/Old.md", "- body\n")]);
    let answer = pages::rename_would_change(
        &store,
        "Old",
        "New",
        &[
            RenameProbeText {
                raw: "tags:: Old".into(),
                org: false,
                pre_block: true,
            },
            RenameProbeText {
                raw: "#+title: x\n#+tags: Old".into(),
                org: true,
                pre_block: true,
            },
            RenameProbeText {
                raw: "see [[file:./Old.org][old]]".into(),
                org: true,
                pre_block: false,
            },
            block("unrelated"),
        ],
    )
    .unwrap();
    assert_eq!(answer, vec![true, true, true, false]);
    let _ = fs::remove_dir_all(&root);
}
