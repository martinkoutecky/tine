//! Martin D2/D4/D7, 2026-10-04. OG text.cljs parse-property;
//! query_dsl.cljs build-block-content; rules.cljc block-content;
//! components/query.cljs table? and query_table.cljs result-table.
use tine_core::query::ir::{Cardinality, ExecutionContext, ObservedType, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::wire_parse::{parse_query_pair, QueryTextDialect};
use tine_core::query::{parse_query_input, QueryInput};
use tine_store::{IrAnswer, IrRequest, Store, WholeGraph};

fn fixture(config: &str) -> (tempfile::TempDir, WholeGraph) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("logseq")).unwrap();
    std::fs::write(dir.path().join("logseq/config.edn"), config).unwrap();
    std::fs::write(dir.path().join("pages/Decisions.md"),
        "list:: Foo, Bar\n\n- Alpha café\n  list:: Foo, Bar\n  authors:: Alice, Bob\n  tags:: Red, Blue\n  alias:: Uno, Dos\n  status:: done\n  n:: 1,5\n- alpha cafe\n- Drawer\n  :LOGBOOK:\n  ExactDrawer\n  :END:\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    (dir, graph)
}
fn raws(graph: &WholeGraph, text: &str, input: QueryInput) -> Vec<String> {
    let (query, view) = parse_query_input(
        text,
        input,
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
        panic!("result")
    };
    let QueryRows::Block { groups } = result.rows else {
        panic!("blocks")
    };
    groups
        .into_iter()
        .flat_map(|g| g.blocks.into_iter().map(|b| b.raw))
        .collect()
}
fn count(graph: &WholeGraph, query: &str) -> usize {
    raws(graph, query, QueryInput::MacroQuery).len()
}
#[test]
fn d2_whole_values_and_configured_atoms_reach_the_query_door() {
    let (_dir, graph) = fixture("{:property/separated-by-commas #{:authors}}");
    assert_eq!(count(&graph, "(property list \"foo, bar\")"), 2);
    assert_eq!(count(&graph, "(property list Foo)"), 0);
    assert_eq!(count(&graph, "(property n 1)"), 0);
    for query in [
        "(property authors Alice)",
        "(property tags Red)",
        "(property alias Dos)",
        "(property status Done)",
    ] {
        assert_eq!(count(&graph, query), 1, "{query}");
    }
    let IrAnswer::Registry(registry) = graph.query_ir(IrRequest::Registry).unwrap() else {
        panic!("registry")
    };
    for key in ["list", "n"] {
        let row = registry.row(key).unwrap();
        assert_eq!(row.cardinality, Cardinality::One, "{key}");
        assert_eq!(row.observed_type, ObservedType::Text, "{key}");
    }
    assert_eq!(
        registry.row("authors").unwrap().cardinality,
        Cardinality::Many
    );
    let (_dir, unconfigured) = fixture("{}");
    assert_eq!(count(&unconfigured, "(property authors Alice)"), 0);
}
#[test]
fn d4_query_text_is_exact_and_sees_raw_property_and_drawer_text() {
    let (_dir, graph) = fixture("{}");
    for (query, expected) in [
        ("Alpha", 1),
        ("\"café\"", 1),
        ("cafe", 1),
        ("\"list:: Foo, Bar\"", 2),
        ("ExactDrawer", 1),
    ] {
        assert_eq!(count(&graph, query), expected, "{query}");
    }
    let search = graph
        .search(
            &tine_store::SearchRequest {
                text: "CAFE".into(),
                within: None,
                page_limit: 0,
                block_limit: 10,
                explain: false,
                page_match_scope: None,
                page_view: None,
                block_view: None,
            },
            &tine_store::Cancel(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                false,
            ))),
        )
        .unwrap();
    assert_eq!(search.hits.len(), 2, "Search still folds case and accents");
    for op in ["like", "match"] {
        for value in ["Alpha", "cafe", "café", "ExactDrawer"] {
            let pattern = if op == "like" {
                format!("%{value}%")
            } else {
                value.to_string()
            };
            let q = format!("@block and content {op} '{pattern}'");
            assert_eq!(raws(&graph, &q, QueryInput::MacroTql).len(), 1, "{q}");
        }
    }
}
#[test]
fn d7_legacy_table_spelling_reaches_the_wire_query_door() {
    for (text, properties) in [
        ("(task TODO)", vec![("query-table".into(), "true".into())]),
        ("(task TODO) table", vec![]),
        ("(task TODO) {:table-view? true}", vec![]),
    ] {
        let parsed = parse_query_pair(
            text,
            QueryTextDialect::MacroQuery,
            &properties,
            Registry::none(),
        );
        assert!(parsed.query.diagnostics.is_empty(), "{text}");
        assert!(parsed.legacy_table, "{text}");
        let (plain, _) = parse_query_input(
            "(task TODO)",
            QueryInput::MacroQuery,
            tine_core::date::JournalDate::today(),
            Registry::none(),
        );
        assert_eq!(
            parsed.query.filter, plain.filter,
            "table is presentation only"
        );
    }
}

#[test]
fn org_query_content_and_properties_use_the_same_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("logseq")).unwrap();
    std::fs::write(
        dir.path().join("logseq/config.edn"),
        "{:feature/enable-search-remove-accents? false :property/separated-by-commas #{:authors}}",
    )
    .unwrap();
    std::fs::write(dir.path().join("pages/Org.org"), "* Alpha café\r\n:PROPERTIES:\r\n:list: Foo, Bar\r\n:authors: Alice, Bob\r\n:END:\r\n* alpha cafe\r\n").unwrap();
    let graph = Store::open(dir.path(), Default::default())
        .unwrap()
        .0
        .whole_graph()
        .unwrap();
    for query in [
        "(property list \"foo, bar\")",
        "(property authors Alice)",
        "Alpha",
        "cafe",
        "\":list: Foo, Bar\"",
    ] {
        assert_eq!(count(&graph, query), 1, "{query}");
    }
    for query in ["(property list Foo)", "\"list:: Foo, Bar\""] {
        assert_eq!(count(&graph, query), 0, "{query}");
    }
    let off = parse_query_pair(
        "(task TODO)",
        QueryTextDialect::MacroQuery,
        &[("query-table".into(), "false".into())],
        Registry::none(),
    );
    assert!(!off.legacy_table);
}
