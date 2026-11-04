//! CM step 2, batch 8 (`REG-CM-PREAMBLE-PROSE-REFERENCES`): text before a
//! page's first block is that page's first block for every reference
//! question, as in OG (graph-parser `block.cljs with-pre-block-if-exists`: the
//! pre-block's `:block/refs` are its properties' and its body's). A link
//! written there is a Linked Reference of its target (a preamble row, not a
//! page-property row), counts in All pages, makes a reference-only page, and
//! a `((uuid))` there counts as a block referrer. Page properties in the same
//! pre-block stay page-property rows.

use crate::Store;
use tine_core::model::BlockDto;

/// The row flag, read off the wire shape (the window reads the same).
fn preamble(b: &BlockDto) -> bool {
    serde_json::to_value(b).unwrap().get("page_preamble") == Some(&serde_json::Value::Bool(true))
}

const ID: &str = "6571a2b3-0000-4000-8000-00000000c8b8";

fn graph() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    let w = |rel: &str, text: &str| std::fs::write(dir.path().join(rel), text).unwrap();
    w("pages/Target.md", &format!("- target\n  id:: {ID}\n"));
    w("pages/Prose.md", "[[Target]] in prose\n");
    w(
        "pages/Mixed.md",
        "tags:: Other\n\nSee [[Target]] and #Ghost here\n\n- [[Target]] in a block\n",
    );
    w(
        "pages/OrgProse.org",
        "#+title: OrgProse\n\nSee [[Target]] before the first heading\n* heading\n",
    );
    w("pages/Fenced.md", "```\n[[Target]]\n```\n\n- body\n");
    w("pages/BlockRef.md", &format!("(({ID})) quoted\n\n- body\n"));
    dir
}

fn check(store: &Store, when: &str) {
    let graph = store.whole_graph().unwrap();
    let linked = graph.backlinks("Target").unwrap();
    let rows = |page: &str| -> Vec<(String, bool, bool)> {
        linked
            .iter()
            .filter(|group| group.page == page)
            .flat_map(|group| group.blocks.iter())
            .map(|b| (b.raw.clone(), b.page_property, preamble(b)))
            .collect()
    };
    assert_eq!(
        rows("Prose"),
        [("[[Target]] in prose".to_owned(), false, true)],
        "{when}: {linked:?}"
    );
    assert_eq!(
        rows("Mixed"),
        [
            ("See [[Target]] and #Ghost here".to_owned(), false, true),
            ("[[Target]] in a block".to_owned(), false, false),
        ],
        "{when}: {linked:?}"
    );
    assert_eq!(
        rows("OrgProse"),
        [(
            "See [[Target]] before the first heading".to_owned(),
            false,
            true
        )],
        "{when}: {linked:?}"
    );
    assert!(rows("Fenced").is_empty(), "{when}: code is not a reference");
    // Every group's evidence names a row it holds.
    for group in linked.iter() {
        assert_eq!(
            group.evidence.len(),
            group.blocks.len(),
            "{when}: {group:?}"
        );
    }

    // The page property in the same pre-block stays a page-property row.
    let other = graph.backlinks("Other").unwrap();
    assert_eq!(other.len(), 1, "{when}: {other:?}");
    assert!(other[0].blocks[0].page_property && !preamble(&other[0].blocks[0]));

    // All pages: the count is the panel's row count; a name only the prose
    // mentions is a reference-only page.
    let (files, references) = graph.page_backlink_counts();
    let count = |rel: &str| files.iter().find(|(path, _)| path == rel).map(|(_, n)| *n);
    assert_eq!(count("pages/Target.md"), Some(4), "{when}: {files:?}");
    assert!(
        references
            .iter()
            .any(|(name, n)| name.eq_ignore_ascii_case("ghost") && *n == 1),
        "{when}: {references:?}"
    );

    // A block reference in the prose is a referrer of that block.
    assert_eq!(graph.block_ref_counts().get(ID).copied(), Some(1), "{when}");
    let referrers = graph.block_referrers(ID).unwrap();
    assert_eq!(referrers.len(), 1, "{when}: {referrers:?}");
    assert!(preamble(&referrers[0].blocks[0]));
}

#[test]
fn prose_before_the_first_block_is_a_referencing_block() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let dir = graph();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    check(&store, "cold");
    store.close();
    // A warm launch answers the same from the checkpoint.
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    check(&store, "warm");
    store.close();
}
