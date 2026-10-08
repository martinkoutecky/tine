use tine_core::query::ir::{Anchor, ExecutionContext, Filter, Query, Source, ViewSettings};
use tine_store::{IrRequest, Store};

#[test]
fn ir_store_refuses_large_answers_with_its_own_limits() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(dir.path().join("pages/Rows.md"), "- row\n".repeat(20_001)).unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    let query = Query::new(Anchor::Block, Filter::True, Source::Builder);
    let answer = graph.query_ir(IrRequest::Run {
        query: &query,
        view: &ViewSettings::default(),
        context: &ExecutionContext::none(),
    });
    let error = tine_core::query::ir::QueryResult::try_from(answer.unwrap())
        .expect_err("The store's wire conversion must refuse an exceeded IR result");
    assert_eq!(error, "result-too-large: 20001 matching rows; narrow the query or add a sample (construction limits: 20000 rows / 33554432 bytes)");
    let sampled = graph
        .query_ir(IrRequest::Run {
            query: &query,
            view: &ViewSettings {
                sample: Some(5),
                ..Default::default()
            },
            context: &ExecutionContext::none(),
        })
        .unwrap();
    let sampled = tine_core::query::ir::QueryResult::try_from(sampled).unwrap();
    assert_eq!(sampled.total, 5);
    assert_eq!(sampled.matched_total, Some(20_001));
}

#[test]
fn ir_refusal_uses_the_complete_count_or_total_and_preserves_small_answers() {
    use tine_core::query::ir::{QueryReport, QueryResult, QueryRows};
    use tine_store::IrAnswer;
    let result = |matched_total, exceeded| QueryResult {
        statistics: None,
        rows: QueryRows::Page { pages: Vec::new() },
        diagnostics: Vec::new(),
        report: QueryReport {
            supported: true,
            ..Default::default()
        },
        total: 2,
        matched_total,
        exceeded,
    };
    for (matched_total, count) in [(Some(43), 43), (None, 2)] {
        let error = QueryResult::try_from(IrAnswer::Result(Box::new(result(matched_total, true))))
            .unwrap_err();
        assert!(
            error.starts_with(&format!("result-too-large: {count} matching rows")),
            "{error}"
        );
    }
    assert_eq!(
        QueryResult::try_from(IrAnswer::Result(Box::new(result(None, false))))
            .unwrap()
            .total,
        2
    );
}

#[test]
fn result_limits_have_one_answerer() {
    let wire = include_str!("../../../src-tauri/src/commands/query_ir.rs");
    assert!(wire.contains("QueryResult::try_from(answer)") && !wire.contains("result-too-large: {"), "I-12: the store's TryFrom<IrAnswer> owns the IR refusal message; the wire only forwards it");
    for name in ["RESULT_BRIDGE_MAX_ROWS", "RESULT_BRIDGE_MAX_BYTES"] {
        assert!(!wire.contains(&format!("const {name}")), "I-12: store.rs owns result bridge limits and refusals; query_ir.rs must not declare a twin limit");
    }
}
