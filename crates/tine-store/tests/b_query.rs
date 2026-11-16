use tine_store::Store;

#[test]
fn b_query_cross_row_atom_uniqueness_and_prefix_matching_reach_the_store() {
    let _integration_guard = crate::integration_lock();
    use tine_core::query::ir::{
        Anchor, Attr, CmpOp, ExecutionContext, Filter, Quant, Query, QueryRows, Rel, Source, Value,
        ViewSettings,
    };
    use tine_store::{IrAnswer, IrRequest};
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    let first = (0..10_000)
        .map(|i| format!("v{i:05}"))
        .collect::<Vec<_>>()
        .join(",");
    let second = (5_000..15_000)
        .map(|i| format!("v{i:05}"))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        dir.path().join("pages/Atoms.md"),
        format!("- alpha\n  tags:: {first}\n  tags:: {second}\n"),
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    let filter = Filter::and(
        [
            Filter::attr(
                Attr::AtomCount,
                CmpOp::Eq,
                Value::Number { number: 15_000.0 },
            ),
            Filter::attr(Attr::Value, CmpOp::Eq, Value::text("v14999")),
        ]
        .into_iter()
        .map(|test| {
            Filter::rel(
                Rel::Props,
                Quant::Any,
                Filter::and(vec![
                    Filter::attr(Attr::Key, CmpOp::Eq, Value::text("tags")),
                    test,
                ]),
            )
        })
        .collect(),
    );
    let query = Query::new(Anchor::Block, filter, Source::Builder);
    let result = graph
        .query_ir(
            IrRequest::Run {
                query: &query,
                view: &ViewSettings::default(),
                context: &ExecutionContext::default(),
            },
            &tine_store::Cancel::default(),
        )
        .map(|answer| answer.expect("never cancelled"))
        .unwrap();
    let IrAnswer::Result(result) = result else {
        panic!("result rows")
    };
    assert!(
        matches!(&result.rows, QueryRows::Block { groups } if groups.len() == 1 && groups[0].blocks.len() == 1)
    );
    let (query, view) = tine_core::query::parse_query_text(
        r"content like '\a%'",
        tine_core::query::QueryDialect::Tql,
        tine_core::date::JournalDate::today(),
    );
    let result = graph
        .query_ir(
            IrRequest::Run {
                query: &query,
                view: &view,
                context: &ExecutionContext::default(),
            },
            &tine_store::Cancel::default(),
        )
        .map(|answer| answer.expect("never cancelled"))
        .unwrap();
    let IrAnswer::Result(result) = result else {
        panic!("result rows")
    };
    assert!(matches!(&result.rows, QueryRows::Block { groups } if groups.len() == 1));
}

#[test]
fn b_query_org_template_metadata_is_removed_by_the_copy_door() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(dir.path().join("pages/Templates.org"), "* Root\n:PROPERTIES:\n:template: Demo\n:template-including-parent: true\n:END:\n** Child\n#+BEGIN_SRC text\n:template: literal\n#+END_SRC\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let templates = store.whole_graph().unwrap().templates();
    let copied = &templates.iter().find(|t| t.name == "Demo").unwrap().blocks[0];
    assert!(!copied.raw.contains(":template:"));
    assert!(!copied.raw.contains(":template-including-parent:"));
    assert!(copied.children[0].raw.contains(":template: literal"));
}
