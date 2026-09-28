//! Master's execution conformance tests (`tine-core/src/query/conformance.rs`
//! at the 0.6.983 line, §3.4 truth table and §4.4 binding) that Q1 deferred,
//! run end to end through og's engine: `Store::open` → `WholeGraph::query_ir`
//! for the IR route and `WholeGraph::query` for the `{{query}}` bridge.
//!
//! Omitted, with reasons: the §8.1 counterfactual-mode matrix (master
//! 717–930: `CompareMode` is master's gate-1 attribution tool, not an
//! execution path og has); the export
//! agreement half of `run_explain_and_export_agree` (og's export path is the
//! bridge, covered in `model.rs`).

use std::path::Path;

use tine_core::date::JournalDate;
use tine_core::model::{PageKind, RefGroup};
use tine_core::query::ir::{
    Anchor, ExecutionContext, ExplainEmptyResult, Query, QueryResult, QueryRows, Source,
    ViewSettings,
};
use tine_core::query::{parse_query_input, parse_query_text, QueryDialect, QueryInput};
use tine_store::{IrAnswer, IrRequest, OpenOptions, Store, WholeGraph};

/// A per-test graph directory, removed on drop.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("tine-query-conformance-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _dir: TempDir,
    graph: WholeGraph,
}

fn open(pages: &[(&str, &str)]) -> Fixture {
    let dir = TempDir::new();
    std::fs::create_dir_all(dir.path().join("pages")).expect("pages");
    std::fs::create_dir_all(dir.path().join("journals")).expect("journals");
    for (path, text) in pages {
        write(dir.path(), path, text);
    }
    let (store, _, _) = Store::open(dir.path(), OpenOptions::default()).expect("open");
    let graph = store.whole_graph().expect("load");
    Fixture { _dir: dir, graph }
}

fn write(root: &Path, path: &str, text: &str) {
    std::fs::write(root.join(path), text).expect("page");
}

/// The §3.4 truth-table graph (master `PAGES` + `JOURNAL`, verbatim).
fn truth_graph() -> Fixture {
    open(&[
        (
            "pages/Rows.md",
            "\
- TODO plain marker
- [#A] priority without a marker
- SCHEDULED: <2026-07-29 Wed>
- DEADLINE: <2026-08-01 Sat>
- TODO marked and scheduled
  SCHEDULED: <2026-07-30 Thu>
- bare SCHEDULED: with no date
- `SCHEDULED: <2026-07-29 Wed>` inside inline code
- malformed planning
  SCHEDULED: <2026-13-45 Xxx>
",
        ),
        (
            "pages/Children.md",
            "\
- leaf parent
- all children done
\t- DONE one
\t- DONE two
- mixed children
\t- DONE one
\t- TODO two
- TODO root that violates the child predicate
",
        ),
        (
            "pages/Props.md",
            "\
- absent property
- blank property
  size::
- numeric property
  size:: 5
- text property
  size:: large
- mixed numeric and text
  size:: 5, large
",
        ),
        ("pages/Names.md", "- a block on a named page\n"),
        (
            "pages/Tagged.md",
            "tags:: alpha, beta\n\n- a block on a tagged page\n",
        ),
        ("pages/Proj%2FSub.md", "- a block under the namespace\n"),
        ("journals/2026_07_29.md", "- a journal block\n"),
    ])
}

fn first_lines(groups: &[RefGroup]) -> Vec<String> {
    let mut out: Vec<String> = groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| {
            block
                .raw
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .collect();
    out.sort();
    out
}

/// The `{{query}}` bridge's rows, sorted.
fn rows(graph: &WholeGraph, query: &str) -> Vec<String> {
    match graph
        .query(query, tine_store::QueryDialect::Simple)
        .expect("query")
    {
        tine_store::QueryResult::Simple(groups) => first_lines(&groups),
        tine_store::QueryResult::Advanced(_) => unreachable!(),
    }
}

fn case(graph: &WholeGraph, query: &str, expected: &[&str]) {
    let mut expected: Vec<String> = expected.iter().map(|text| text.to_string()).collect();
    expected.sort();
    assert_eq!(rows(graph, query), expected, "{query}");
}

fn run_ir(
    graph: &WholeGraph,
    query: &Query,
    view: &ViewSettings,
    context: &ExecutionContext,
) -> QueryResult {
    match graph.query_ir(IrRequest::Run {
        query,
        view,
        context,
    }) {
        Ok(IrAnswer::Result(result)) => *result,
        other => panic!("{other:?}"),
    }
}

fn explain(graph: &WholeGraph, query: &Query, context: &ExecutionContext) -> ExplainEmptyResult {
    match graph.query_ir(IrRequest::ExplainEmpty { query, context }) {
        Ok(IrAnswer::ExplainEmpty(explained)) => explained,
        other => panic!("{other:?}"),
    }
}

/// `source` in `dialect`, run through the IR route with no current page.
fn run_text(graph: &WholeGraph, source: &str, dialect: QueryDialect) -> QueryResult {
    let (query, view) = parse_query_text(source, dialect, JournalDate::today());
    assert!(!query.is_invalid(), "{source}: {:?}", query.diagnostics);
    run_ir(graph, &query, &view, &ExecutionContext::none())
}

fn block_lines(result: &QueryResult) -> Vec<String> {
    let QueryRows::Block { groups } = &result.rows else {
        panic!(
            "a block-anchored query returns block rows: {:?}",
            result.rows
        );
    };
    first_lines(groups)
}

fn page_names(result: &QueryResult) -> Vec<String> {
    let QueryRows::Page { pages } = &result.rows else {
        panic!("a page-anchored query returns page rows: {:?}", result.rows);
    };
    pages.iter().map(|page| page.name.clone()).collect()
}

#[test]
fn optional_block_attributes_are_two_valued() {
    let fixture = truth_graph();
    let graph = &fixture.graph;
    case(
        graph,
        "(task TODO)",
        &[
            "TODO plain marker",
            "TODO marked and scheduled",
            "TODO root that violates the child predicate",
            "TODO two",
        ],
    );
    case(graph, "(priority A)", &["[#A] priority without a marker"]);
    // og deviation (Martin's inline-planning model, f5f514878): a `SCHEDULED:
    // <date>` anywhere in the block counts, so the one inside inline code is
    // a row here; master excludes it. The bare `SCHEDULED:` has no date and
    // is not a row in either.
    case(
        graph,
        "(scheduled)",
        &[
            "SCHEDULED: <2026-07-29 Wed>",
            "TODO marked and scheduled",
            "`SCHEDULED: <2026-07-29 Wed>` inside inline code",
            "malformed planning",
        ],
    );
    case(graph, "(deadline)", &["DEADLINE: <2026-08-01 Sat>"]);
    case(
        graph,
        "(between scheduled 2026-07-29 2026-07-31)",
        &[
            "SCHEDULED: <2026-07-29 Wed>",
            "TODO marked and scheduled",
            "`SCHEDULED: <2026-07-29 Wed>` inside inline code",
        ],
    );
}

#[test]
fn property_leaves_are_two_valued_over_atoms() {
    let fixture = truth_graph();
    let graph = &fixture.graph;
    case(
        graph,
        "(property size)",
        &[
            "blank property",
            "numeric property",
            "text property",
            "mixed numeric and text",
        ],
    );
    case(
        graph,
        "(property size 5)",
        &["numeric property", "mixed numeric and text"],
    );
    case(graph, "(property missing anything)", &[]);
}

#[test]
fn a_leaf_block_satisfies_every_over_its_empty_children() {
    let fixture = truth_graph();
    let (query, _) = parse_query_text(
        "every(children, task = 'DONE')",
        QueryDialect::Tql,
        JournalDate::today(),
    );
    assert!(!query.is_invalid(), "{:?}", query.diagnostics);
    // Printed and re-parsed on purpose: the case is the round trip.
    let printed = tine_core::query::print::print_tql(&query);
    let matched = block_lines(&run_text(&fixture.graph, &printed, QueryDialect::Tql));
    assert!(matched.contains(&"leaf parent".to_string()), "{matched:?}");
    assert!(
        matched.contains(&"all children done".to_string()),
        "{matched:?}"
    );
    assert!(
        !matched.contains(&"mixed children".to_string()),
        "{matched:?}"
    );
}

#[test]
fn any_over_children_is_false_on_a_leaf_block() {
    let fixture = truth_graph();
    let result = run_text(
        &fixture.graph,
        "any(children, task = 'TODO')",
        QueryDialect::Tql,
    );
    assert_eq!(block_lines(&result), vec!["mixed children".to_string()]);
}

#[test]
fn the_page_anchor_returns_page_rows() {
    let fixture = truth_graph();
    let (query, _) = parse_query_text(
        "@page and name like 'proj/%'",
        QueryDialect::Tql,
        JournalDate::today(),
    );
    assert_eq!(query.anchor, Anchor::Page);
    let result = run_text(
        &fixture.graph,
        "@page and name like 'proj/%'",
        QueryDialect::Tql,
    );
    assert_eq!(page_names(&result), vec!["Proj/Sub"]);
    let result = run_text(
        &fixture.graph,
        "@page and name like 'proj%'",
        QueryDialect::Tql,
    );
    assert_eq!(page_names(&result).len(), 1, "{:?}", result.rows);
}

#[test]
fn the_journal_page_attribute_is_a_page_row_leaf() {
    let fixture = truth_graph();
    let result = run_text(
        &fixture.graph,
        "@page and journal = true",
        QueryDialect::Tql,
    );
    assert_eq!(page_names(&result), vec!["Jul 29th, 2026"]);
}

/// I-12 as a test: the OG and TQL spellings of one filter return the same rows.
#[test]
fn the_two_dialects_agree_row_for_row() {
    let fixture = truth_graph();
    let graph = &fixture.graph;
    for (og, tql) in [
        ("(task TODO)", "task = 'TODO'"),
        ("(priority A)", "priority = 'A'"),
        ("(property size 5)", "prop('size') = '5'"),
        ("(page Names)", "page.name = 'Names'"),
        ("(namespace Proj)", "page.name like 'proj/%'"),
        ("(scheduled)", "scheduled is not null"),
        ("(deadline)", "deadline is not null"),
    ] {
        let tql_rows = block_lines(&run_text(graph, tql, QueryDialect::Tql));
        assert_eq!(rows(graph, og), tql_rows, "{og} vs {tql}");
    }
}

/// REG-P0-QUERY-ALL-PAGE-TAGS-001.
#[test]
fn all_page_tags_selects_every_page_carrying_a_tag() {
    let fixture = truth_graph();
    let result = run_text(&fixture.graph, "(all-page-tags)", QueryDialect::Og);
    assert_eq!(page_names(&result), vec!["Tagged"]);
    // The block-group bridge answers with that page's blocks.
    case(
        &fixture.graph,
        "(all-page-tags)",
        &["a block on a tagged page"],
    );
}

/// REG-P0-QUERY-UNKNOWN-HEAD-001.
#[test]
fn an_unknown_head_returns_nothing_rather_than_a_shorter_query() {
    let fixture = truth_graph();
    assert_eq!(rows(&fixture.graph, "(task TODO)").len(), 4);
    case(&fixture.graph, "(and (task TODO) (frobnicate x))", &[]);
}

/// A property atom is coerced by its key's effective type (§6.3), not by how
/// the query spells its literal.
#[test]
fn property_atoms_compare_by_their_keys_effective_type() {
    let fixture = open(&[(
        "pages/Typed.md",
        "\
- score ten
  score:: 10
- score nine
  score:: 9
- score is a word
  score:: high
- due in august
  due:: 2026-08-05
- due in september
  due:: 2026-09-01
- due someday
  due:: someday
",
    )]);
    let typed = |query: &str| block_lines(&run_text(&fixture.graph, query, QueryDialect::Tql));
    assert_eq!(typed("prop('score') > 9"), vec!["score ten"]);
    assert_eq!(typed("prop('score') != 9"), vec!["score ten"]);
    // og deviation (OG `(property k v)` compares text): a text literal equals
    // an atom's exact text even on a number key; `high` is such an atom.
    assert_eq!(typed("prop('score') = 'high'"), vec!["score is a word"]);
    assert_eq!(
        typed("prop('score') is not null"),
        vec!["score is a word", "score nine", "score ten"]
    );
    assert_eq!(
        typed("every(prop('score'), value > 3)"),
        vec!["score nine", "score ten"]
    );
    assert_eq!(typed("prop('due') < '2026-08-15'"), vec!["due in august"]);
    assert_eq!(
        typed("prop('due') between '2026-08-01' and '2026-09-30'"),
        vec!["due in august", "due in september"]
    );
    assert_eq!(
        typed("prop('due') is not null"),
        vec!["due in august", "due in september", "due someday"]
    );
}

/// Master's journal-first tie: two groups of the same name order journal
/// first. og classifies a page by its directory (v0.6.5), so unlike master's
/// Direct Files the pair IS reachable from disk and the rule is exercised
/// end to end.
#[test]
fn two_groups_of_the_same_name_are_ordered_journal_first() {
    let fixture = open(&[
        ("journals/2026_07_29.md", "- TODO from the journal\n"),
        ("pages/Jul 29th, 2026.md", "- TODO from the named page\n"),
    ]);
    let tine_store::QueryResult::Simple(groups) = fixture
        .graph
        .query("(task TODO)", tine_store::QueryDialect::Simple)
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        groups
            .iter()
            .map(|group| (group.page.as_str(), group.kind))
            .collect::<Vec<_>>(),
        vec![
            ("Jul 29th, 2026", PageKind::Journal),
            ("Jul 29th, 2026", PageKind::Page),
        ]
    );
}

/// SPEC §3.2's measured planning fixture: OG stores no planning day for a
/// `SCHEDULED:` on the marker's own line.
#[test]
fn planning_on_the_markers_own_line() {
    let fixture = open(&[(
        "pages/Planning.md",
        "- TODO SCHEDULED: <2026-08-05 Wed>\n- TODO on its own\n  SCHEDULED: <2026-08-06 Thu>\n",
    )]);
    // og deviation (f5f514878, Martin's inline-planning model): og counts the
    // inline timestamp, so both blocks are scheduled rows; master (and OG's
    // measured `:block/scheduled 0`) answer only the second.
    assert_eq!(
        rows(&fixture.graph, "(scheduled)"),
        vec!["TODO SCHEDULED: <2026-08-05 Wed>", "TODO on its own"]
    );
}

// --- §4.4 binding ---------------------------------------------------------

const CURRENT_PAGE_QUERY: &str = "{:query [:find (pull ?b [*]) \
:in $ ?p :where [?page :block/name ?p] [?b :block/refs ?page]] \
:inputs [:current-page]}";

fn binding_graph() -> Fixture {
    open(&[
        ("pages/Alpha.md", "- alpha body\n"),
        ("pages/Beta.md", "- beta body\n"),
        (
            "pages/Links.md",
            "- links to [[Alpha]]\n- links to [[Beta]] and more\n",
        ),
    ])
}

fn advanced_query(source: &str) -> Query {
    parse_query_input(
        source,
        QueryInput::Advanced,
        JournalDate::today(),
        tine_core::query::registry::Registry::none(),
    )
    .0
}

#[test]
fn one_parse_answers_differently_on_two_current_pages() {
    let fixture = binding_graph();
    let graph = &fixture.graph;
    let query = advanced_query(CURRENT_PAGE_QUERY);
    let Source::Advanced { original, .. } = &query.source else {
        panic!("{:?}", query.source);
    };
    assert_eq!(original, CURRENT_PAGE_QUERY);
    let view = ViewSettings::default();
    let on_alpha = run_ir(graph, &query, &view, &ExecutionContext::on_page("Alpha"));
    let on_beta = run_ir(graph, &query, &view, &ExecutionContext::on_page("Beta"));
    assert_eq!(
        block_lines(&on_beta),
        vec!["beta body", "links to [[Beta]] and more"]
    );
    assert_eq!(
        block_lines(&on_alpha),
        vec!["alpha body", "links to [[Alpha]]"]
    );
    assert!(on_alpha.report.supported && on_beta.report.supported);
    assert!(on_alpha
        .report
        .ran
        .contains(&"current-page-ref".to_string()));
    let again = run_ir(graph, &query, &view, &ExecutionContext::on_page("Alpha"));
    assert_eq!(block_lines(&again), block_lines(&on_alpha));
}

#[test]
fn a_missing_runtime_input_is_strict_no_results_with_a_report() {
    let fixture = binding_graph();
    let query = advanced_query(CURRENT_PAGE_QUERY);
    let result = run_ir(
        &fixture.graph,
        &query,
        &ViewSettings::default(),
        &ExecutionContext::none(),
    );
    assert!(block_lines(&result).is_empty(), "{:?}", result.rows);
    assert!(!result.report.supported, "{:?}", result.report);
    assert_eq!(result.total, 0);
}

#[test]
fn a_relative_date_query_answers_for_the_execution_day_across_a_rollover() {
    let query = advanced_query("[:find (pull ?b [*]) :where (between ?b -7d today)]");
    let context = ExecutionContext::none();
    let thursday = JournalDate {
        year: 2026,
        month: 9,
        day: 4,
    };
    let friday = JournalDate {
        year: 2026,
        month: 9,
        day: 5,
    };
    let before = tine_core::query::resolve_for_execution(&query, &context, thursday);
    let after = tine_core::query::resolve_for_execution(&query, &context, friday);
    assert_eq!(before.today(), thursday);
    assert_eq!(after.today(), friday);
    assert_ne!(before.query().filter, after.query().filter);
}

#[test]
fn run_and_explain_agree_on_results_and_report() {
    let fixture = binding_graph();
    let query = advanced_query(CURRENT_PAGE_QUERY);
    let context = ExecutionContext::on_page("Beta");
    let run = run_ir(&fixture.graph, &query, &ViewSettings::default(), &context);
    let explained = explain(&fixture.graph, &query, &context);
    assert_eq!(run.report, explained.report);
    assert_eq!(run.total, 2, "{:?}", run.rows);
    assert_eq!(
        block_lines(&run),
        vec!["beta body", "links to [[Beta]] and more"]
    );
    // The explanation counts the BOUND tree, not the advanced placeholder.
    assert_eq!(explained.rows.len(), 1, "{:?}", explained.rows);
    assert_eq!(explained.rows[0].alone, run.total);
}

#[test]
fn explain_empty_reports_a_failed_resolution_without_misleading_counts() {
    let fixture = binding_graph();
    let query = advanced_query(CURRENT_PAGE_QUERY);
    let explained = explain(&fixture.graph, &query, &ExecutionContext::none());
    assert!(explained.rows.is_empty(), "{:?}", explained.rows);
    assert!(!explained.report.supported);
    assert!(
        !explained.diagnostics.is_empty(),
        "a refused binding still says why"
    );
}

/// I-20 at the store: an answer never survives an edit that changes it, and
/// an unrelated edit keeps it (memoized, scoped invalidation).
#[test]
fn an_edit_moves_the_answer_and_an_unrelated_edit_keeps_it() {
    let dir = TempDir::new();
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    write(dir.path(), "pages/A.md", "- TODO first\n");
    write(dir.path(), "pages/B.md", "- unrelated\n");
    let (store, _, _) = Store::open(dir.path(), OpenOptions::default()).expect("open");
    let (query, view) = parse_query_text("(task TODO)", QueryDialect::Og, JournalDate::today());
    let answer = |store: &Store| {
        block_lines(&run_ir(
            &store.whole_graph().unwrap(),
            &query,
            &view,
            &ExecutionContext::none(),
        ))
    };
    assert_eq!(answer(&store), vec!["TODO first"]);
    // Replace the first block's text through the ordinary save path.
    let save = |path: &str, raw: &str| {
        let id = tine_store::PageId::from(path);
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = raw.to_string();
        assert!(matches!(
            store.save(
                tine_store::EditKind::SaveBlock,
                &id,
                tine_store::SaveBase::Existing(read.rev),
                &doc
            ),
            tine_store::SaveOutcome::Saved(_)
        ));
    };
    save("pages/B.md", "still unrelated");
    assert_eq!(answer(&store), vec!["TODO first"]);
    save("pages/A.md", "DONE first");
    assert!(answer(&store).is_empty());
}

/// G3 (og 14 Q2) I-22/I-25: a `LIKE` pattern at the admitted source limit
/// over long blocks answers through the real entry point within a bound. The
/// old per-comparison matcher cost O(block × pattern) (~8e9 steps per block
/// here) and did not answer.
#[test]
fn an_admitted_limit_like_pattern_over_long_blocks_answers_within_a_bound() {
    let block = format!("{}c", "a".repeat(128 * 1024));
    let page: String = (0..8).map(|_| format!("- {block}\n")).collect();
    let fixture = open(&[
        ("pages/Long.md", &page),
        ("pages/Short.md", "- a short c\n"),
    ]);
    let run = "a".repeat(65_000);
    let source = format!("content like '%{run}b'");
    assert!(source.len() <= tine_core::query::QUERY_SOURCE_MAX_BYTES);
    let (tx, rx) = std::sync::mpsc::channel();
    let graph = fixture.graph.clone();
    std::thread::spawn(move || {
        let miss = block_lines(&run_text(&graph, &source, QueryDialect::Tql));
        let hit = block_lines(&run_text(
            &graph,
            &format!("content like '%{}c'", "a".repeat(65_000)),
            QueryDialect::Tql,
        ));
        let _ = tx.send((miss, hit));
    });
    let (miss, hit) = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("an admitted LIKE pattern must answer within its bound (I-22)");
    assert!(miss.is_empty());
    assert_eq!(hit.len(), 8);
}
