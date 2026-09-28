//! The IR query commands (master SPEC §7.1 names and shapes): `query_parse`,
//! `query_print`, `query_og_expressible`, `query_registry`, `query_run` and
//! `query_explain_empty`. Every graph read runs on the blocking pool; og's
//! command errors are strings.

use std::sync::Arc;

use tine_core::query::ir::{
    ExecutionContext, ExplainEmptyResult, Query, QueryResult, RegistrySnapshot, ViewSettings,
};
use tine_core::query::registry::Registry;
pub(crate) use tine_core::query::wire_parse::{parse_query_pair, ParsedQuery, QueryTextDialect};
use tine_store::{IrAnswer, IrRequest, WholeGraph};

use crate::state::{slot_for_context, GraphContext};

/// Mirrors `tine-store`'s result bridge limits, quoted in the refusal text.
const RESULT_BRIDGE_MAX_ROWS: usize = 20_000;
const RESULT_BRIDGE_MAX_BYTES: usize = 32 * 1024 * 1024;

/// The printed form a `query_print` caller wants (SPEC §4.3, §7.1).
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueryPrintDialect {
    /// OG DSL, the argument of a `{{query …}}` macro (wrapper not included).
    Og,
    /// The text pane's multi-line editing layout.
    Tql,
    /// The argument of the persisted single-line `{{tine-query …}}` macro
    /// (wrapper not included).
    TqlMacro,
    /// The argument of a `{{query [:find …]}}` advanced macro, printed from its
    /// authored source (wrapper not included).
    AdvancedMacro,
}

fn validate_query_source(query: &str) -> Result<(), String> {
    if !tine_core::query::query_source_within_limit(query) {
        return Err(format!(
            "query-too-large: query source is {} bytes (limit: {} bytes)",
            query.len(),
            tine_core::query::QUERY_SOURCE_MAX_BYTES
        ));
    }
    if !tine_core::query::query_nesting_within_limit(query) {
        return Err("query-nesting-too-deep: query nesting exceeds the parser limit".into());
    }
    Ok(())
}

fn print_query_text(
    query: &Query,
    view: &ViewSettings,
    dialect: QueryPrintDialect,
    preserve_form: bool,
) -> Result<String, String> {
    use tine_core::query::print::PrintDialect;
    let dialect = match dialect {
        QueryPrintDialect::Og => PrintDialect::Og,
        QueryPrintDialect::Tql => PrintDialect::Tql,
        QueryPrintDialect::TqlMacro => PrintDialect::TqlMacro,
        QueryPrintDialect::AdvancedMacro => PrintDialect::AdvancedMacro,
    };
    tine_core::query::print::query_print(query, view, dialect, preserve_form).map_err(
        |diagnostic| {
            let reason = match diagnostic.kind {
                tine_core::query::ir::DiagnosticKind::NotApplicable => "not_applicable",
                _ => "syntax",
            };
            // The diagnostic travels as structure (JSON), never as prose.
            format!(
                "query-print-refused:{reason}:{}",
                serde_json::to_string(&diagnostic).unwrap_or_default()
            )
        },
    )
}

/// A result the WebView cannot be handed is a refusal, not a truncation.
fn query_result_or_error(result: QueryResult) -> Result<QueryResult, String> {
    if result.exceeded {
        let complete = result.matched_total.unwrap_or(result.total);
        return Err(format!(
            "result-too-large: {complete} matching rows; narrow the query or add a sample (construction limits: {RESULT_BRIDGE_MAX_ROWS} rows / {RESULT_BRIDGE_MAX_BYTES} bytes)"
        ));
    }
    Ok(result)
}

fn registry(graph: &WholeGraph) -> Result<Arc<Registry>, String> {
    match graph.query_ir(IrRequest::Registry) {
        Ok(IrAnswer::Registry(registry)) => Ok(registry),
        Ok(_) => Err("query-registry: unexpected answer".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn parse(
    graph: &WholeGraph,
    text: &str,
    dialect: QueryTextDialect,
    block_properties: &[(String, String)],
) -> Result<ParsedQuery, String> {
    validate_query_source(text)?;
    Ok(parse_query_pair(
        text,
        dialect,
        block_properties,
        &*registry(graph)?,
    ))
}

fn run(
    graph: &WholeGraph,
    query: &Query,
    view: &ViewSettings,
    context: &ExecutionContext,
) -> Result<QueryResult, String> {
    match graph.query_ir(IrRequest::Run {
        query,
        view,
        context,
    }) {
        Ok(IrAnswer::Result(result)) => query_result_or_error(*result),
        Ok(_) => Err("query-run: unexpected answer".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn explain_empty(
    graph: &WholeGraph,
    query: &Query,
    context: &ExecutionContext,
) -> Result<ExplainEmptyResult, String> {
    match graph.query_ir(IrRequest::ExplainEmpty { query, context }) {
        Ok(IrAnswer::ExplainEmpty(explained)) => Ok(explained),
        Ok(_) => Err("query-explain-empty: unexpected answer".into()),
        Err(error) => Err(error.to_string()),
    }
}

async fn on_graph<T: Send + 'static>(
    state: &GraphContext<'_>,
    read: impl FnOnce(&WholeGraph) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let slot = slot_for_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let graph = slot
            .store
            .whole_graph()
            .map_err(|e| format!("graph load failed: {e:?}"))?;
        read(&graph)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// SPEC §7.1 `query_parse`: text → `{query, view}` (plus the scoped display
/// settings), with the host block's `tine.*` properties merged here and nowhere
/// else (a `tine.*` value wins per field; other keys are ignored). A syntax
/// error is NOT an `Err`: it comes back in `query.diagnostics`. `Err` only for
/// a source over the size/nesting limit, or a graph that is unbound, closed or
/// failed to load. Waits for the initial graph load; the first call per
/// registry generation builds the property registry, O(P + B).
#[tauri::command]
pub(crate) async fn query_parse(
    text: String,
    dialect: QueryTextDialect,
    block_properties: Option<Vec<(String, String)>>,
    state: GraphContext<'_>,
) -> Result<ParsedQuery, String> {
    validate_query_source(&text)?;
    on_graph(&state, move |graph| {
        parse(
            graph,
            &text,
            dialect,
            block_properties.as_deref().unwrap_or_default(),
        )
    })
    .await
}

/// SPEC §7.1 `query_print`: the IR as the MACRO ARGUMENT (no `{{…}}`) for
/// `og`/`tql_macro`/`advanced_macro`, or the text pane's layout for `tql`.
/// Refusals (`Err("query-print-refused:<reason>:<diagnostic JSON>")`): `og` when
/// the IR is not OG-expressible; any macro dialect whose output fails the
/// macro-safety / document-parser check; `advanced_macro`, or
/// `preserve_form = true`, when the query's source is not that dialect's
/// (`tql` always refuses `preserve_form`). `preserve_form` re-emits the
/// authored source verbatim plus its options map. View: `og` prints only a
/// single-key sort and `sample`; aggregates, grouping, columns and the view
/// kind are NOT printed and must be persisted as `tine.*` properties by the
/// caller; `tql`/`tql_macro` ignore the view. Pure; no graph access.
#[tauri::command]
pub(crate) async fn query_print(
    query: Query,
    view: ViewSettings,
    dialect: QueryPrintDialect,
    preserve_form: Option<bool>,
) -> Result<String, String> {
    print_query_text(&query, &view, dialect, preserve_form.unwrap_or(false))
}

/// SPEC §7.1 `query_og_expressible`: whether the OG DSL can express this
/// filter and view (false only for an OG-inexpressible filter or a multi-key
/// sort). A precondition of `query_print(og)`, not a guarantee: the printer can
/// still refuse text that fails the macro-safety check. Pure.
#[tauri::command]
pub(crate) async fn query_og_expressible(query: Query, view: ViewSettings) -> bool {
    tine_core::query::print::og_expressible(&query, &view)
}

/// SPEC §7.1 `query_registry`: the observed property registry (§6.1), at most
/// 8 top values per key. Waits for the initial graph load; `Err` only for a
/// graph that is unbound, closed or failed to load.
#[tauri::command]
pub(crate) async fn query_registry(state: GraphContext<'_>) -> Result<RegistrySnapshot, String> {
    on_graph(&state, |graph| {
        registry(graph).map(|registry| registry.snapshot())
    })
    .await
}

/// SPEC §7.1 `query_run`: the parsed IR, evaluated in memory on the latest
/// published snapshot (waits for the initial load). An over-bound answer
/// (20,000 rows / 32 MiB, checked after `sample` for both anchors) is refused
/// with `Err("result-too-large: …")`, never truncated. An invalid query is `Ok`
/// with no rows and its `diagnostics`. `context` binds only an advanced
/// source's typed `:current-page` input; without it that clause is dropped and
/// listed in `report.ignored`, so the answer is broader. A statistics fold over
/// budget is an `Err`.
#[tauri::command]
pub(crate) async fn query_run(
    query: Query,
    view: ViewSettings,
    context: Option<ExecutionContext>,
    state: GraphContext<'_>,
) -> Result<QueryResult, String> {
    let context = context.unwrap_or_default();
    on_graph(&state, move |graph| run(graph, &query, &view, &context)).await
}

/// SPEC §7.1 `query_explain_empty` (N19): why the query returned nothing — one
/// row per root conjunct with its count alone and without it (one row, with no
/// `without` count, for a non-`And` root; none for a non-executable query).
/// The view is accepted for master's command shape and ignored: probes count
/// rows unsorted and unsampled. Cost O(conjuncts × (P + B)); not memoized.
/// Waits for the initial graph load.
#[tauri::command]
pub(crate) async fn query_explain_empty(
    query: Query,
    view: ViewSettings,
    context: Option<ExecutionContext>,
    state: GraphContext<'_>,
) -> Result<ExplainEmptyResult, String> {
    let _ = view;
    let context = context.unwrap_or_default();
    on_graph(&state, move |graph| explain_empty(graph, &query, &context)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_free_registry() -> Registry {
        Registry::from_snapshot(&RegistrySnapshot {
            rows: Vec::new(),
            generation: 0,
        })
    }

    fn parsed(text: &str, dialect: QueryTextDialect) -> ParsedQuery {
        parse_query_pair(text, dialect, &[], &graph_free_registry())
    }

    fn graph_with(files: &[(&str, &str)]) -> (tempfile::TempDir, WholeGraph) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let (store, _, _) =
            tine_store::Store::open(dir.path(), tine_store::OpenOptions::default()).unwrap();
        let graph = store.whole_graph().unwrap();
        (dir, graph)
    }

    fn parse_and_run(graph: &WholeGraph, text: &str, dialect: QueryTextDialect) -> Vec<String> {
        let parsed = parse(graph, text, dialect, &[]).unwrap();
        assert!(!parsed.query.is_invalid(), "{:?}", parsed.query.diagnostics);
        let result = run(
            graph,
            &parsed.query,
            &parsed.view,
            &ExecutionContext::none(),
        )
        .unwrap();
        match result.rows {
            tine_core::query::ir::QueryRows::Page { pages } => {
                pages.into_iter().map(|row| row.name).collect()
            }
            tine_core::query::ir::QueryRows::Block { groups } => groups
                .into_iter()
                .flat_map(|group| group.blocks)
                .map(|block| block.raw.lines().next().unwrap_or("").to_string())
                .collect(),
        }
    }

    #[test]
    fn query_parse_returns_the_pair_for_both_dialects() {
        let og = parsed("(and (task TODO) [[Project]])", QueryTextDialect::Og);
        assert!(!og.query.is_invalid(), "{:?}", og.query.diagnostics);
        let tql = parsed("task in ('TODO') and [[Project]]", QueryTextDialect::Tql);
        assert!(!tql.query.is_invalid(), "{:?}", tql.query.diagnostics);
        assert_eq!(og.query.normalized().filter, tql.query.normalized().filter);
    }

    #[test]
    fn query_parse_merges_the_host_blocks_view_properties() {
        let merged = parse_query_pair(
            "(and (task TODO) (sort-by page asc))",
            QueryTextDialect::Og,
            &[("tine.sample".to_string(), "5".to_string())],
            &graph_free_registry(),
        );
        assert_eq!(merged.view.sample, Some(5));
        assert!(!merged.view.sort.is_empty());
    }

    #[test]
    fn query_print_rejects_a_non_og_expressible_ir_with_the_diagnostic() {
        let og = parsed("(and (task TODO) [[Project]])", QueryTextDialect::Og);
        let printed = print_query_text(&og.query, &og.view, QueryPrintDialect::Og, false).unwrap();
        assert!(printed.starts_with('('), "{printed}");
        assert!(tine_core::query::print::og_expressible(&og.query, &og.view));
        let tql_only = parsed("any(children, task = 'DONE')", QueryTextDialect::Tql);
        assert!(
            !tql_only.query.is_invalid(),
            "{:?}",
            tql_only.query.diagnostics
        );
        assert!(!tine_core::query::print::og_expressible(
            &tql_only.query,
            &tql_only.view
        ));
        let error = print_query_text(
            &tql_only.query,
            &tql_only.view,
            QueryPrintDialect::Og,
            false,
        )
        .expect_err("the OG printer is partial");
        assert!(
            error.starts_with("query-print-refused:not_applicable:{"),
            "{error}"
        );
    }

    #[test]
    fn query_run_refuses_an_over_budget_result_rather_than_truncating_it() {
        let result = QueryResult {
            statistics: None,
            rows: tine_core::query::ir::QueryRows::Page { pages: Vec::new() },
            diagnostics: Vec::new(),
            report: tine_core::query::ir::QueryReport {
                supported: true,
                ..Default::default()
            },
            total: 2,
            matched_total: Some(43),
            exceeded: true,
        };
        let error = query_result_or_error(result).expect_err("an exceeded result is a refusal");
        assert!(
            error.starts_with("result-too-large: 43 matching rows"),
            "{error}"
        );
    }

    #[test]
    fn query_run_answers_page_and_block_rows_through_the_store() {
        let (_dir, graph) = graph_with(&[
            ("pages/Proj%2FSub.md", "- under a namespace\n"),
            (
                "pages/Other.md",
                "- elsewhere\n- TODO a task [[Proj/Sub]]\n",
            ),
        ]);
        assert_eq!(
            parse_and_run(
                &graph,
                "@page and name like 'proj/%'",
                QueryTextDialect::Tql
            ),
            vec!["Proj/Sub".to_string()]
        );
        assert_eq!(
            parse_and_run(
                &graph,
                "(and (task TODO) [[Proj/Sub]])",
                QueryTextDialect::Og
            ),
            vec!["TODO a task [[Proj/Sub]]".to_string()]
        );
    }

    #[test]
    fn query_explain_empty_answers_per_conjunct_for_a_root_and() {
        let (_dir, graph) = graph_with(&[(
            "pages/Explain.md",
            "- TODO a task with no project\n- a project mention [[Project]]\n",
        )]);
        let parsed = parse(
            &graph,
            "(and (task TODO) [[Project]])",
            QueryTextDialect::Og,
            &[],
        )
        .unwrap();
        let explained = explain_empty(&graph, &parsed.query, &ExecutionContext::none()).unwrap();
        assert_eq!(explained.rows.len(), 2, "{:?}", explained.rows);
        assert!(explained.rows.iter().all(|line| line.without.is_some()));
        assert!(
            explained.rows.iter().all(|line| line.alone == 1),
            "{:?}",
            explained.rows
        );
    }

    #[test]
    fn query_registry_reports_observed_properties() {
        let (_dir, graph) = graph_with(&[("pages/A.md", "- item\n  status:: open\n")]);
        let snapshot = registry(&graph).unwrap().snapshot();
        assert!(
            snapshot
                .rows
                .iter()
                .any(|row| row.normalized_name == "status"),
            "{snapshot:?}"
        );
        let typo = parse(&graph, "statuss = 'x'", QueryTextDialect::Tql, &[]).unwrap();
        assert!(typo
            .query
            .diagnostics
            .iter()
            .any(|d| d.suggestions.contains(&"prop('status')".to_string())));
    }

    /// The macro UI moved from `run_query` / `run_advanced_query` to
    /// `query_parse` + `query_run`; a block-anchored source answers the same
    /// rows, in the same order, either way.
    #[test]
    fn macro_sources_answer_the_same_rows_through_query_run_as_through_the_bridge() {
        let (_dir, graph) = graph_with(&[
            (
                "pages/Alpha.md",
                "- TODO [#A] first [[Project]]\n  status:: open\n- DONE second\n- LATER third #tag\n",
            ),
            ("pages/Project.md", "- NOW fourth\n  status:: closed\n- plain [[Alpha]]\n"),
        ]);
        let first_lines = |groups: &[tine_core::model::RefGroup]| -> Vec<String> {
            groups
                .iter()
                .flat_map(|group| group.blocks.iter())
                .map(|block| block.raw.lines().next().unwrap_or("").to_string())
                .collect()
        };
        let through_run = |source: &str| {
            let parsed = parse(&graph, source, QueryTextDialect::MacroQuery, &[]).unwrap();
            let result = run(
                &graph,
                &parsed.query,
                &parsed.view,
                &ExecutionContext::none(),
            )
            .unwrap();
            let rows = match &result.rows {
                tine_core::query::ir::QueryRows::Block { groups } => first_lines(groups),
                // An invalid source answers no rows under either anchor.
                tine_core::query::ir::QueryRows::Page { pages } if pages.is_empty() => Vec::new(),
                tine_core::query::ir::QueryRows::Page { .. } => {
                    panic!("{source}: block rows expected")
                }
            };
            (rows, result.report)
        };
        for source in [
            "(task TODO LATER NOW)",
            "[[Project]]",
            "(and (task TODO DONE) [[Project]])",
            "(or #tag (property status closed))",
            "(and (task TODO LATER NOW) (sort-by priority desc))",
            "(not (task DONE))",
            "(frobnicate x)",
        ] {
            let tine_store::QueryResult::Simple(bridge) = graph
                .query(source, tine_store::QueryDialect::Simple)
                .unwrap()
            else {
                unreachable!()
            };
            assert_eq!(through_run(source).0, first_lines(&bridge), "{source}");
        }
        for source in [
            "[:find (pull ?b [*]) :where [?b :block/marker ?m] [(contains? #{\"TODO\" \"NOW\"} ?m)]]",
            "[:find (pull ?b [*]) :where (task ?b #{\"DONE\"})]",
            "[:find ?x :where [?x :no/such ?y] [(foo ?y)]]",
        ] {
            let tine_store::QueryResult::Advanced(bridge) =
                graph.query(source, tine_store::QueryDialect::Advanced).unwrap()
            else {
                unreachable!()
            };
            let (rows, report) = through_run(source);
            assert_eq!(rows, first_lines(&bridge.groups), "{source}");
            assert_eq!(
                (report.ran, report.ignored, report.supported),
                (bridge.ran.clone(), bridge.ignored.clone(), bridge.supported),
                "{source}"
            );
        }
    }

    /// #301: `:inputs [:current-page]` (BEGIN_QUERY's execution source) binds
    /// the page the caller passes as `context` (the query block passes OG's
    /// current page: focused route, else home, else today); with no page bound
    /// it answers nothing.
    #[test]
    fn current_page_binds_the_rendering_page() {
        let (_dir, graph) = graph_with(&[
            ("pages/Focus A.md", "- on a\n"),
            ("pages/Focus B.md", "- on b\n"),
            (
                "pages/Notes.md",
                "- TODO about [[Focus A]]\n- TODO about [[Focus B]]\n",
            ),
        ]);
        let rows = |source: &str, page: Option<&str>| {
            let parsed = parse(&graph, source, QueryTextDialect::MacroQuery, &[]).unwrap();
            let context = ExecutionContext {
                current_page: page.map(str::to_string),
            };
            match run(&graph, &parsed.query, &parsed.view, &context)
                .unwrap()
                .rows
            {
                tine_core::query::ir::QueryRows::Block { groups } => groups
                    .into_iter()
                    .flat_map(|group| group.blocks)
                    .map(|block| block.raw.lines().next().unwrap_or("").to_string())
                    .collect::<Vec<_>>(),
                tine_core::query::ir::QueryRows::Page { pages } => {
                    pages.into_iter().map(|row| row.name).collect()
                }
            }
        };
        let advanced = "[:find (pull ?b [*]) :in $ ?current-page :where [?p :block/name ?current-page] [?b :block/refs ?p]] :inputs [:current-page]";
        // Q1 lowers `[?b :block/refs ?p]` to the page-ref filter (master's
        // lowering), whose path-refs closure includes the page's own blocks.
        assert_eq!(
            rows(advanced, Some("Focus A")),
            vec!["on a", "TODO about [[Focus A]]"]
        );
        assert_eq!(
            rows(advanced, Some("Focus B")),
            vec!["on b", "TODO about [[Focus B]]"]
        );
        assert!(rows(advanced, None).is_empty());
    }

    #[test]
    fn every_graph_reading_query_command_crosses_the_blocking_pool() {
        let source = include_str!("query_ir.rs");
        let body = &source[source.find("async fn on_graph").unwrap()..];
        assert!(body[..body.find("\n}\n").unwrap()].contains("spawn_blocking"));
        for name in [
            "query_parse",
            "query_registry",
            "query_run",
            "query_explain_empty",
        ] {
            let start = source
                .find(&format!("pub(crate) async fn {name}("))
                .expect("command stays async");
            let tail = &source[start..];
            let end = tail.find("\n}\n").unwrap();
            assert!(
                tail[..end].contains("on_graph("),
                "{name} must read through on_graph"
            );
        }
    }
}
