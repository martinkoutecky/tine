use super::*;

fn open() -> (tempfile::TempDir, crate::Store) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    for n in 0..30 {
        std::fs::write(dir.path().join(format!("pages/P{n}.md")), "- unrelated\n").unwrap();
    }
    std::fs::write(
        dir.path().join("pages/Chosen.md"),
        "- one\n- two\n- three\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("pages/Other.md"), "- other\n").unwrap();
    let store = crate::Store::open(dir.path(), Default::default())
        .unwrap()
        .0;
    store.whole_graph().unwrap();
    (dir, store)
}

#[test]
fn page_scoped_simple_and_tql_queries_build_no_unrelated_page_facts() {
    for tql in [false, true] {
        let (_dir, store) = open();
        let graph = store.whole_graph().unwrap();
        crate::query::index::BUILT_FACT_PAGES.with(|count| count.set(0));
        crate::query::index::DERIVED_FACT_PAGES.with(|count| count.set(0));
        if tql {
            let (query, view) = parse_query_text(
                "@block and page.name = 'Chosen'",
                QueryDialect::Tql,
                JournalDate::today(),
            );
            let context = ExecutionContext::none();
            let crate::IrAnswer::Result(answer) = graph
                .query_ir(crate::IrRequest::Run {
                    query: &query,
                    view: &view,
                    context: &context,
                })
                .unwrap()
            else {
                panic!("expected result")
            };
            assert_eq!(answer.total, 3);
        } else {
            let crate::QueryResult::Simple(groups) = graph
                .query("(page [[Chosen]])", crate::QueryDialect::Simple)
                .unwrap()
            else {
                panic!("expected simple result")
            };
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].page, "Chosen");
            assert_eq!(groups[0].blocks.len(), 3);
        }
        let work = crate::query::index::BUILT_FACT_PAGES.with(|count| count.get());
        let derived = crate::query::index::DERIVED_FACT_PAGES.with(|count| count.get());
        store.close();
        assert_eq!(work, 0, "I-13/I-25: one-page queries must not derive graph-wide facts; exemplar query/exec_candidates.rs (TQL={tql})");
        assert_eq!(derived, 1, "one selected page's facts (TQL={tql})");
    }
}

#[test]
fn page_constraints_preserve_or_not_refs_duplicates_and_page_rows() {
    let (dir, store) = open();
    std::fs::create_dir(dir.path().join("pages/nested")).unwrap();
    std::fs::write(
        dir.path().join("pages/nested/Chosen.md"),
        "- duplicate [[Other]]\n",
    )
    .unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    let graph = store.whole_graph().unwrap();
    for (source, rows) in [
        (
            "@block and (page.name = 'Chosen' or page.name = 'Other')",
            5,
        ),
        (
            "@block and (page.name = 'Chosen' or content like '%unrelated%')",
            34,
        ),
        ("@block and not page.name = 'Chosen'", 31),
        ("@block and [[Other]]", 2),
        ("@page and name = 'Chosen'", 2),
        ("@block and page.name = 'Absent'", 0),
    ] {
        let (query, view) = parse_query_text(source, QueryDialect::Tql, JournalDate::today());
        assert!(
            query.diagnostics.is_empty(),
            "{source}: {:?}",
            query.diagnostics
        );
        let context = ExecutionContext::none();
        let crate::IrAnswer::Result(answer) = graph
            .query_ir(crate::IrRequest::Run {
                query: &query,
                view: &view,
                context: &context,
            })
            .unwrap()
        else {
            panic!("expected result")
        };
        assert_eq!(answer.total, rows, "{source}");
    }
    store.close();
}
