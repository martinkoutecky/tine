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
                .query_ir(
                    crate::IrRequest::Run {
                        query: &query,
                        view: &view,
                        context: &context,
                    },
                    &crate::Cancel::default(),
                )
                .map(|answer| answer.expect("never cancelled"))
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
            .query_ir(
                crate::IrRequest::Run {
                    query: &query,
                    view: &view,
                    context: &context,
                },
                &crate::Cancel::default(),
            )
            .map(|answer| answer.expect("never cancelled"))
            .unwrap()
        else {
            panic!("expected result")
        };
        assert_eq!(answer.total, rows, "{source}");
    }
    store.close();
}

/// Card BhkrU's cost line: `any(direct_refs, day > today)` walks only the pages
/// whose reference set holds a name denoting a later day; every other page is
/// skipped from its facts alone, and the narrowing never changes the answer
/// (the `or false` twin defeats it and must agree).
#[test]
fn bhkru_future_day_condition_walks_only_pages_that_mention_such_a_day() {
    let (dir, store) = open();
    let today = JournalDate::today();
    std::fs::write(
        dir.path().join("pages/Dated.md"),
        format!(
            "- TODO soon [[{}]]\n- past [[{}]]\n- 2048 has digits but [[no day 42]]\n",
            today.add_days(2).title(),
            today.add_days(-2).title()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("pages/Past.md"),
        format!("- only behind us [[{}]]\n", today.add_days(-9).title()),
    )
    .unwrap();
    store.refresh(crate::Depth::Stamps).unwrap();
    let whole = store.whole_graph().unwrap();
    let graph: &ReadSnapshot = &whole.graph;
    let mut walked = Vec::new();
    for source in [
        "any(direct_refs, day > today)",
        "task = 'TODO' and any(direct_refs, day between today and '+7d')",
    ] {
        let (query, _) = parse_query_text(source, QueryDialect::Tql, today);
        assert!(
            query.diagnostics.is_empty(),
            "{source}: {:?}",
            query.diagnostics
        );
        let plan = plan(graph, &query, today, false);
        graph.with_pages(|pages| {
            let candidates = candidates::Candidates::new(graph, pages, &plan);
            let atoms = EvalCache::default();
            let mut names: Vec<String> = candidates
                .pages()
                .filter(|(entry, doc)| {
                    !plan.skips(
                        entry,
                        &candidates.facts(entry, doc),
                        candidates.config(),
                        &atoms,
                    )
                })
                .map(|(entry, _)| entry.name.clone())
                .collect();
            names.sort();
            walked.push(names);
        });
    }
    assert_eq!(walked, vec![vec!["Dated".to_owned()]; 2]);
    for (narrowed, unnarrowed, rows) in [
        (
            "any(direct_refs, day > today)",
            "any(direct_refs, day > today) or false",
            1,
        ),
        (
            "none(direct_refs, journal = true) and content like '%unrelated%'",
            "none(direct_refs, journal = true) and content like '%unrelated%' or false",
            30,
        ),
    ] {
        let mut totals = Vec::new();
        for source in [narrowed, unnarrowed] {
            let (query, view) = parse_query_text(source, QueryDialect::Tql, today);
            assert!(
                query.diagnostics.is_empty(),
                "{source}: {:?}",
                query.diagnostics
            );
            let context = ExecutionContext::none();
            let crate::IrAnswer::Result(answer) = whole
                .query_ir(
                    crate::IrRequest::Run {
                        query: &query,
                        view: &view,
                        context: &context,
                    },
                    &crate::Cancel::default(),
                )
                .map(|answer| answer.expect("never cancelled"))
                .unwrap()
            else {
                panic!("expected result")
            };
            totals.push(answer.total);
        }
        assert_eq!(totals, vec![rows, rows], "{narrowed}");
    }
    store.close();
}
