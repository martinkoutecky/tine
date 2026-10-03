//! GH Discussion #617: `{{query (property type [[Person]])}}` returned nothing
//! for pages whose `type:: [[Person]]` is a HEADER page property.
//!
//! OG provenance (read-only checkout, src/main/frontend/db/query_dsl.cljs and
//! deps/db/src/logseq/db/rules.cljc): a header-style page property block is a
//! real block (`:block/pre-block? true`) carrying `:block/properties`, so the
//! block-anchored simple-query predicates (`property`, `page`, `between`, page
//! refs, full text) match it like any other block; `custom-query-result-
//! transform` does not filter pre-blocks out. Tine keeps the header text in
//! `Document::pre_block`, outside `roots`, so block evaluation never saw it.
//!
//! The fixture goes through the same path a `{{query}}` block takes
//! (`parse_query_input(.., MacroQuery, ..)` then `query_ir(Run)`).

use std::collections::BTreeSet;

use tine_core::model::BlockDto;
use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, QueryInput};
use tine_store::{IrAnswer, IrRequest, Store, WholeGraph};

/// `(page name, block)` rows of a block-anchored macro query, in result order.
fn rows(graph: &WholeGraph, q: &str) -> Vec<(String, BlockDto)> {
    let (query, view) = parse_query_input(
        q,
        QueryInput::MacroQuery,
        tine_core::date::JournalDate::today(),
        Registry::none(),
    );
    let IrAnswer::Result(result) = graph
        .query_ir(IrRequest::Run {
            query: &query,
            view: &view,
            context: &ExecutionContext::default(),
        })
        .unwrap()
    else {
        panic!("query_ir(Run) returns a result");
    };
    let QueryRows::Block { groups } = result.rows else {
        panic!("`{q}` is block-anchored");
    };
    groups
        .into_iter()
        .flat_map(|g| g.blocks.into_iter().map(move |b| (g.page.clone(), b)))
        .collect()
}

fn pages(graph: &WholeGraph, q: &str) -> BTreeSet<String> {
    rows(graph, q).into_iter().map(|(page, _)| page).collect()
}

fn fixture() -> (tempfile::TempDir, WholeGraph) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("journals")).unwrap();
    let w = |name: &str, text: &str| {
        let path = if name.starts_with("journals/") {
            dir.path().join(name)
        } else {
            dir.path().join("pages").join(name)
        };
        std::fs::write(path, text).unwrap();
    };
    // Header-style (page-level) `type::`, every value spelling.
    w("PageLink.md", "type:: [[Person]]\n\n- body\n");
    w("PagePlain.md", "type:: Person\n\n- body\n");
    w("PageTag.md", "type:: #Person\n\n- body\n");
    w("PageMulti.md", "type:: [[Person]], [[Agent]]\n\n- body\n");
    // Block-level `type::` (always worked).
    w("BlockLink.md", "- hello\n  type:: [[Person]]\n");
    w("BlockPlain.md", "- hello2\n  type:: Person\n");
    w(
        "BlockMulti.md",
        "- hello3\n  type:: [[Person]], [[Agent]]\n",
    );
    w("Other.md", "type:: [[Place]]\n\n- body\n");
    w("Person.md", "- the person page\n");
    w(
        "Tagged.md",
        "tags:: research, [[Idea]]\nstatus:: draft\n\n- TODO task one\n- body two\n",
    );
    w("journals/2020_12_06.md", "mood:: good\n\n- jbody\n");
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    (dir, graph)
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn property_query_matches_header_page_properties_in_every_value_spelling() {
    let (_dir, graph) = fixture();
    let expected = set(&[
        "PageLink",
        "PagePlain",
        "PageTag",
        "PageMulti",
        "BlockLink",
        "BlockPlain",
        "BlockMulti",
    ]);
    for q in [
        "(property type [[Person]])",
        "(property type Person)",
        "(property type \"Person\")",
        "(property type #Person)",
    ] {
        assert_eq!(pages(&graph, q), expected, "{q}");
    }
    assert_eq!(
        pages(&graph, "(property type [[Agent]])"),
        set(&["PageMulti", "BlockMulti"])
    );
    // Key-only form: every block (pre-blocks included) that has the key.
    assert_eq!(
        pages(&graph, "(property type)"),
        expected
            .iter()
            .cloned()
            .chain(["Other".to_string()])
            .collect()
    );
}

#[test]
fn header_property_hit_is_the_read_only_page_property_row() {
    let (_dir, graph) = fixture();
    let hits = rows(&graph, "(property type [[Person]])");
    let header = hits.iter().find(|(page, _)| page == "PageLink").unwrap();
    assert!(
        header.1.page_property,
        "synthetic page-property row, not an editable block"
    );
    assert!(
        header.1.raw.contains("type:: [[Person]]"),
        "{:?}",
        header.1.raw
    );
    let block = hits.iter().find(|(page, _)| page == "BlockLink").unwrap();
    assert!(!block.1.page_property);
}

#[test]
fn every_block_anchored_predicate_sees_the_pre_block() {
    let (_dir, graph) = fixture();
    // Tags and arbitrary page properties.
    assert_eq!(pages(&graph, "(property tags research)"), set(&["Tagged"]));
    assert_eq!(pages(&graph, "(property status draft)"), set(&["Tagged"]));
    // Journal header property, and `between` (the pre-block is a block of the journal).
    assert_eq!(
        pages(&graph, "(property mood good)"),
        set(&["Dec 6th, 2020"])
    );
    assert!(
        pages(&graph, "(between [[Dec 5th, 2020]] [[Dec 7th, 2020]])").contains("Dec 6th, 2020")
    );
    assert!(
        rows(&graph, "(between [[Dec 5th, 2020]] [[Dec 7th, 2020]])")
            .iter()
            .any(|(_, b)| b.page_property)
    );
    // `page`: the pre-block is a block of the page.
    let page = rows(&graph, "(page Tagged)");
    assert_eq!(page.len(), 3, "pre-block + two bullets");
    assert!(page[0].1.page_property);
    // Page refs: `type:: [[Person]]` references Person from the pre-block.
    let refs = pages(&graph, "[[Person]]");
    assert!(
        refs.contains("PageLink") && refs.contains("BlockLink"),
        "{refs:?}"
    );
    // Boolean combinators over the pre-block.
    assert_eq!(
        pages(&graph, "(or (property status draft) (property mood good))"),
        set(&["Tagged", "Dec 6th, 2020"])
    );
    // `not` keeps the page's ordinary bullets but drops its matching header block.
    let negated = rows(&graph, "(not (property type Person))");
    assert!(negated
        .iter()
        .any(|(page, b)| page == "PageLink" && !b.page_property));
    assert!(!negated
        .iter()
        .any(|(page, b)| page == "PageLink" && b.page_property));
    assert!(negated
        .iter()
        .any(|(page, b)| page == "Other" && b.page_property));
    // Task/priority never match a property-only pre-block.
    assert_eq!(pages(&graph, "(task TODO)"), set(&["Tagged"]));
    assert!(rows(&graph, "(task TODO)")
        .iter()
        .all(|(_, b)| !b.page_property));
}
