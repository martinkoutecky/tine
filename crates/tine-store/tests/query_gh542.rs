//! GH #542: advanced queries written as DataScript attribute patterns, run end
//! to end over a real graph through og's one advanced entry point
//! (`WholeGraph::query(.., Advanced)`), with master's own fixture and
//! expectations (`model_tests_advanced_queries.rs`, master c1b14a859).
//!
//! QA1: only master's join-free subset is supported. `gh542_joins_are_not_lowered`
//! pins the OUT side of that boundary: a variable shared with another clause is
//! disclosed as unsupported, never answered as a narrower query.

use std::path::{Path, PathBuf};

use tine_core::query::AdvancedResult;
use tine_store::{OpenOptions, QueryDialect, QueryResult, Store, WholeGraph};

const TASKS: &str = "\
- TODO plain
- NOW doing it
- DOING in progress
- LATER someday
  SCHEDULED: <2026-06-25 Thu>
- TODO far future
  SCHEDULED: <2099-01-01 Thu>
- TODO with deadline
  DEADLINE: <2026-06-30 Tue>
- DONE finished
- TODO routine chore
  tags:: Routine
  SCHEDULED: <2026-06-24 Wed>
- no marker here
";

struct Graph {
    dir: PathBuf,
    graph: WholeGraph,
    _store: Store,
}

impl Drop for Graph {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn open(name: &str, pages: &[(&str, &str)]) -> Graph {
    let dir = crate::test_support::fresh_dir(&format!("tine-gh542-{name}-"));
    for sub in ["pages", "journals", "logseq"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    for (path, text) in pages {
        std::fs::write(Path::new(&dir).join(path), text).unwrap();
    }
    let (store, _, _) = Store::open(&dir, OpenOptions::default()).expect("open");
    let graph = store.whole_graph().expect("load");
    Graph {
        dir,
        graph,
        _store: store,
    }
}

fn answer(graph: &Graph, query: &str) -> (Vec<String>, AdvancedResult) {
    let QueryResult::Advanced(result) = graph
        .graph
        .query(query, QueryDialect::Advanced)
        .expect("advanced query")
    else {
        panic!("advanced result")
    };
    let mut raws: Vec<String> = result
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| block.raw.lines().next().unwrap_or("").to_owned())
        .collect();
    raws.sort();
    (raws, result)
}

fn sorted(items: &[&str]) -> Vec<String> {
    let mut items: Vec<String> = items.iter().map(|item| (*item).to_owned()).collect();
    items.sort();
    items
}

fn tasks_graph(name: &str) -> Graph {
    open(
        name,
        &[
            ("journals/2026_06_20.md", TASKS),
            ("pages/Note.md", "- TODO not a journal\n- NOW page now\n"),
        ],
    )
}

#[test]
fn gh542_reporter_advanced_queries_answer_like_logseq() {
    let _integration_guard = crate::integration_lock();
    let graph = tasks_graph("reporter");

    let (raws, result) = answer(
        &graph,
        r#"{:title "Recent tasks"
            :query [:find (pull ?b [*])
                    :where [?b :block/marker "TODO"]]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert_eq!(
        raws,
        sorted(&[
            "TODO plain",
            "TODO far future",
            "TODO with deadline",
            "TODO routine chore",
            "TODO not a journal",
        ])
    );

    // Any marker at all.
    let (raws, result) = answer(
        &graph,
        r#"{:title "Open work" :query [:find (pull ?b [*]) :where [?b :block/marker ?m]]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert_eq!(raws.len(), 10, "{raws:?}");
    assert!(!raws.contains(&"no marker here".to_owned()));

    // `not=` keeps every other marker, and still only blocks that have one.
    let (raws, result) = answer(
        &graph,
        r#"[:find (pull ?b [*]) :where [?b :block/marker ?m] [(not= ?m "DONE")]]"#,
    );
    assert!(result.ignored.is_empty(), "{:?}", result.ignored);
    assert_eq!(raws.len(), 9, "{raws:?}");
    assert!(!raws.contains(&"DONE finished".to_owned()));

    // NOW: a marker variable narrowed by `contains?`.
    let now_query = r#":query [:find (pull ?h [*])
                    :where
                    [?h :block/marker ?marker]
                    [(contains? #{"NOW" "DOING"} ?marker)]]"#;
    let (raws, result) = answer(&graph, &format!(r#"{{:title "NOW" {now_query}}}"#));
    assert!(result.supported, "{:?}", result.ignored);
    assert!(result.ignored.is_empty(), "{:?}", result.ignored);
    assert_eq!(
        raws,
        sorted(&["NOW doing it", "DOING in progress", "NOW page now"])
    );

    // The same query with OG's shipped NOW transform runs: the transform only
    // orders, and Tine runs that exact sort (Martin 2026-10-11). Any other
    // transform is still refused whole (Martin 2026-10-03).
    let (raws, result) = answer(
        &graph,
        &format!(
            r#"{{:title "NOW" {now_query}
            :result-transform (fn [result] (sort-by (fn [h] (get h :block/priority "Z")) result))}}"#
        ),
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert!(result.ignored.is_empty(), "{:?}", result.ignored);
    assert_eq!(
        raws,
        sorted(&["NOW doing it", "DOING in progress", "NOW page now"])
    );
    let (raws, result) = answer(
        &graph,
        &format!(r#"{{:title "NOW" {now_query} :result-transform (fn [xs] (take 1 xs))}}"#),
    );
    assert!(!result.supported);
    assert!(raws.is_empty(), "{raws:?}");
    assert_eq!(result.ignored, vec!["result-transform".to_owned()]);

    // NEXT, with its result variable matching its clauses: tasks on journal
    // pages that have a journal day.
    let (raws, result) = answer(
        &graph,
        r#"{:query [:find (pull ?h [*])
                    :where
                    [?h :block/marker ?marker]
                    [(contains? #{"NOW" "LATER" "TODO"} ?marker)]
                    [?h :block/page ?p]
                    [?p :block/journal? true]
                    [?p :block/journal-day ?d]]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert!(result.ignored.is_empty(), "{:?}", result.ignored);
    assert_eq!(
        raws,
        sorted(&[
            "TODO plain",
            "NOW doing it",
            "LATER someday",
            "TODO far future",
            "TODO with deadline",
            "TODO routine chore",
        ])
    );

    // As the reporter wrote NEXT, it pulls `?b` while every clause binds `?h`,
    // which Logseq refuses too (`?b` is never bound). It must not answer.
    let (raws, result) = answer(
        &graph,
        r#"{:query [:find (pull ?b [*])
                    :where
                    [?h :block/marker ?marker]
                    [(contains? #{"NOW" "LATER" "TODO"} ?marker)]]}"#,
    );
    assert!(!result.supported);
    assert!(raws.is_empty());

    // Scheduled: scheduled on or before today, excluding Routine.
    let (raws, result) = answer(
        &graph,
        r#"{:query [:find (pull ?b [*])
                    :in $ ?next
                    :where
                    (task ?b #{"NOW" "LATER" "TODO" "DOING" "WAIT" "WAITING"})
                    (not (property ?b :tags "Routine"))
                    (or-join [?b ?d]
                             [?b :block/scheduled ?d])
                    [(<= ?d ?next)]]
            :inputs [:today]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert!(result.ignored.is_empty(), "{:?}", result.ignored);
    assert_eq!(raws, sorted(&["LATER someday"]));

    // OVERDUE: NOW/TODO with neither a schedule nor a deadline. The reporter's
    // `(not [?b :block/scheduled ?d])` used to be dropped, listing scheduled
    // tasks too.
    let (raws, result) = answer(
        &graph,
        r#"{:query [:find (pull ?b [*])
                    :where
                    (task ?b #{"NOW" "TODO"})
                    (not [?b :block/scheduled ?d])
                    (not [?b :block/deadline ?d])]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert!(result.ignored.is_empty(), "{:?}", result.ignored);
    assert_eq!(
        raws,
        sorted(&[
            "TODO plain",
            "NOW doing it",
            "TODO not a journal",
            "NOW page now"
        ])
    );
}

/// A clause og only partly understands refuses the whole query (Martin,
/// 2026-10-03): a `not` of a narrower clause removes blocks the query keeps,
/// an `or` of fewer branches drops blocks the query returns, and a plain
/// unknown clause beside a lowered one would broaden the answer. None of them
/// may run as the part Tine understood.
#[test]
fn gh542_a_clause_tine_cannot_lower_refuses_the_whole_query() {
    let _integration_guard = crate::integration_lock();
    let graph = open("partial", &[("journals/2026_06_20.md", TASKS)]);

    for query in [
        r#"[:find (pull ?b [*])
            :where (task ?b #{"TODO"}) (not (and (task ?b #{"TODO"}) (bogus ?b)))]"#,
        r#"[:find (pull ?b [*]) :where (task ?b #{"TODO" "DONE"}) (or (task ?b #{"DONE"}) (bogus ?b))]"#,
        // The audit's repro (2026-10-02, finding 1): a content join beside a
        // supported marker clause answered every TODO block as one group.
        r#"[:find (pull ?b [*]) :where [?b :block/marker "TODO"] [?b :block/content ?c]
            [(clojure.string/includes? ?c "plain")]]"#,
        // An unbound input (`:current-block`) leaves the clause that reads it
        // unevaluable.
        r#"{:query [:find (pull ?b [*]) :in $ ?cb :where [?b :block/marker "TODO"] [?b :block/parent ?cb]]
            :inputs [:current-block]}"#,
    ] {
        let (raws, result) = answer(&graph, query);
        assert!(!result.supported, "{query}: ran={:?}", result.ran);
        assert!(raws.is_empty(), "{query}: {raws:?}");
        assert!(!result.ignored.is_empty(), "{query}");
        assert!(result.ran.is_empty(), "{query}: {:?}", result.ran);
    }
}

/// QA1's OUT side: a variable shared with another clause is a join. og does not
/// lower it (Datalog is never planned), and says so instead of answering a
/// narrower or wider question.
#[test]
fn gh542_joins_are_not_lowered() {
    let _integration_guard = crate::integration_lock();
    let graph = tasks_graph("join");
    let (raws, result) = answer(
        &graph,
        r#"[:find (pull ?b [*])
            :where [?b :block/marker ?m] [?b :block/scheduled ?d] [(<= ?d ?m)]]"#,
    );
    assert!(
        !result.supported || !result.ignored.is_empty(),
        "a join must be disclosed: supported={} ignored={:?} rows={raws:?}",
        result.supported,
        result.ignored
    );
}

/// OG's default NOW query orders its rows by priority, a missing one as "Z"
/// (`config.edn` `:default-queries`, Martin 2026-10-11): A before B before
/// none, stable among equals.
#[test]
fn ogs_now_transform_orders_by_priority_with_missing_last() {
    let _integration_guard = crate::integration_lock();
    let graph = open(
        "now-order",
        &[(
            "pages/Now.md",
            "- NOW c no priority\n- NOW [#B] b first\n- DOING [#A] a\n- NOW [#B] b second\n",
        )],
    );
    let QueryResult::Advanced(result) = graph
        .graph
        .query(
            r#"{:title "NOW"
                :query [:find (pull ?h [*])
                        :where [?h :block/marker ?marker]
                               [(contains? #{"NOW" "DOING"} ?marker)]]
                :result-transform (fn [result] (sort-by (fn [h] (get h :block/priority "Z")) result))}"#,
            QueryDialect::Advanced,
        )
        .expect("advanced query")
    else {
        panic!("advanced result")
    };
    assert!(result.supported, "{:?}", result.ignored);
    let order: Vec<&str> = result
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| block.raw.lines().next().unwrap_or(""))
        .collect();
    assert_eq!(
        order,
        [
            "DOING [#A] a",
            "NOW [#B] b first",
            "NOW [#B] b second",
            "NOW c no priority"
        ]
    );
}

#[test]
fn gh852_reporter_juxt_sorts_run_through_the_rendered_query_path() {
    use tine_core::query::ir::{ExecutionContext, QueryRows};
    use tine_core::query::registry::Registry;
    use tine_core::query::wire_parse::{parse_query_pair, QueryTextDialect};
    let _integration_guard = crate::integration_lock();
    let graph = open(
        "gh852",
        &[(
            "pages/Tasks.md",
            concat!(
                "- TODO [#B] beta\n",
                "- TODO [#A] z scheduled\n  SCHEDULED: <2026-11-07 Sat>\n",
                "- TODO [#A] a deadline\n  DEADLINE: <2026-11-06 Fri>\n",
                "- TODO [#A] c missing\n",
                "- TODO [#A] b both\n  SCHEDULED: <2026-11-07 Sat>\n  DEADLINE: <2026-11-01 Sun>\n",
                "- TODO no priority\n",
            ),
        )],
    );
    let first = r#"(fn [result] (sort-by (juxt (fn [h] (get h :block/priority "Z")) (fn [h] (get h :block/content))) result))"#;
    let second = r#"(fn [result] (sort-by (juxt (fn [h] (get h :block/priority "Z"))
                            (fn [h] (get h :block/scheduled (get h :block/deadline)))
                            (fn [h] (get h :block/content))) result))"#;
    for (transform, expected) in [
        (
            first,
            vec![
                "TODO [#A] a deadline",
                "TODO [#A] b both",
                "TODO [#A] c missing",
                "TODO [#A] z scheduled",
                "TODO [#B] beta",
                "TODO no priority",
            ],
        ),
        (
            second,
            vec![
                "TODO [#A] c missing",
                "TODO [#A] a deadline",
                "TODO [#A] b both",
                "TODO [#A] z scheduled",
                "TODO [#B] beta",
                "TODO no priority",
            ],
        ),
    ] {
        let src = format!(
            r#"{{:query [:find (pull ?h [*]) :where [?h :block/marker "TODO"]] :result-transform {transform}}}"#
        );
        // The parsed query + view boundary is the one used by query_run in a rendered macro.
        let parsed = parse_query_pair(&src, QueryTextDialect::MacroQuery, &[], Registry::none());
        let (query, view) = (parsed.query, parsed.view);
        let tine_store::IrAnswer::Result(result) = graph
            .graph
            .query_ir(
                tine_store::IrRequest::Run {
                    query: &query,
                    view: &view,
                    context: &ExecutionContext::none(),
                },
                &tine_store::Cancel::default(),
            )
            .unwrap()
            .unwrap()
        else {
            panic!("query result")
        };
        assert!(result.report.supported, "{:?}", result.report);
        let QueryRows::Block { groups } = result.rows else {
            panic!("block rows")
        };
        let actual: Vec<_> = groups
            .iter()
            .flat_map(|g| &g.blocks)
            .map(|b| b.raw.lines().next().unwrap())
            .collect();
        assert_eq!(actual, expected);

        // The legacy advanced bridge must produce the same order.
        let QueryResult::Advanced(result) =
            graph.graph.query(&src, QueryDialect::Advanced).unwrap()
        else {
            panic!("advanced")
        };
        assert!(result.supported, "{:?}", result.ignored);
        assert_eq!(
            result
                .groups
                .iter()
                .flat_map(|g| &g.blocks)
                .map(|b| b.raw.lines().next().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
    }

    let unsupported = first.replace(":block/content", ":block/unknown");
    let src = format!(
        r#"{{:query [:find (pull ?h [*]) :where [?h :block/marker "TODO"]] :result-transform {unsupported}}}"#
    );
    let QueryResult::Advanced(result) = graph.graph.query(&src, QueryDialect::Advanced).unwrap()
    else {
        panic!("advanced")
    };
    assert!(!result.supported);
    assert!(
        result.groups.is_empty(),
        "no results with only the first sort key understood"
    );
    assert_eq!(result.ignored, ["result-transform"]);
}

#[test]
fn gh852_content_sort_reads_properties_case_and_utf16_through_the_store() {
    let _integration_guard = crate::integration_lock();
    let graph = open(
        "gh852-content",
        &[(
            "pages/Text.md",
            concat!(
                "- TODO same\n  prop:: z\n",
                "- TODO same\n  prop:: a\n",
                "- TODO \u{e000}\n",
                "- TODO 😀\n",
                "- TODO a\n",
                "- TODO Z\n",
            ),
        )],
    );
    let QueryResult::Advanced(result) = graph.graph.query(
        r#"{:query [:find (pull ?h [*]) :where [?h :block/marker "TODO"]] :result-transform (fn [r] (sort-by (fn [h] (get h :block/content)) r))}"#,
        QueryDialect::Advanced,
    ).unwrap() else { panic!("advanced") };
    assert!(result.supported, "{:?}", result.ignored);
    let raws: Vec<_> = result
        .groups
        .iter()
        .flat_map(|g| &g.blocks)
        .map(|b| b.raw.trim_end())
        .collect();
    assert_eq!(
        raws,
        [
            "TODO Z",
            "TODO a",
            "TODO same\nprop:: a",
            "TODO same\nprop:: z",
            "TODO 😀",
            "TODO \u{e000}"
        ]
    );
}

#[test]
fn gh690_stock_next_uses_direct_refs_and_one_exclusive_date_range() {
    let _integration_guard = crate::integration_lock();
    let today = tine_core::date::JournalDate::today();
    let title = |offset| today.add_days(offset).title();
    let tasks = format!("- TODO inside [[{}]]\n- NOW start [[{}]]\n- LATER end [[{}]]\n- TODO split [[{}]] [[{}]]\n- DONE closed [[{}]]\n- Parent [[{}]]\n  - TODO inherited only\n- TODO named [[Ordinary]]\n", title(3), title(0), title(14), title(-1), title(15), title(3), title(3));
    let graph = open("gh690", &[("pages/Tasks.md", &tasks)]);
    let (raws, result) = answer(
        &graph,
        r#"{:title "📅 NEXT"
        :query [:find (pull ?h [*]) :in $ ?start ?next :where
        [?h :block/marker ?marker] [?h :block/ref-pages ?p]
        [?p :block/journal? true] [?p :block/journal-day ?d]
        [(> ?d ?start)] [(< ?d ?next)]
        [(contains? #{"NOW" "LATER" "TODO"} ?marker)]]
        :inputs [:today :14d-after] :collapsed? false}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert_eq!(raws, vec![format!("TODO inside [[{}]]", title(3))]);
}

#[test]
fn gh690_referenced_journals_follow_title_format_before_safe_defaults() {
    let _integration_guard = crate::integration_lock();
    let graph = open(
        "gh690-formats",
        &[
            (
                "logseq/config.edn",
                r#"{:journal/page-title-format "yyyy_dd_MM" :journal/file-name-format "dd.MM.yyyy"}"#,
            ),
            (
                "pages/Tasks.md",
                "- TODO title [[2026_10_11]]\n- TODO filename only [[10.11.2026]]\n",
            ),
        ],
    );
    let (raws, result) = answer(
        &graph,
        r#"{:query [:find (pull ?h [*]) :in $ ?start ?next :where
        [?h :block/ref-pages ?p] [?p :block/journal? true] [?p :block/journal-day ?d]
        [(> ?d ?start)] [(< ?d ?next)]] :inputs [20261101 20261120]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    // D-20261102-JDL (Martin): the configured file-name format is a date
    // spelling too, a deliberate deviation from OG's title-only list, so the
    // query's journal facts agree with what `[[10.11.2026]]` opens.
    assert_eq!(
        raws,
        [
            "TODO filename only [[10.11.2026]]",
            "TODO title [[2026_10_11]]"
        ]
    );
}

/// GH #1017: the reporter's "Incoming and overdue" `:where`.
const GH1017_WHERE: &str = r#"[?h :block/marker ?m]
    [(contains? #{"LATER" "TODO"} ?m)]
    (or-join [?h ?d]
      (and
        (or [?h :block/page ?p] [?h :block/ref-pages ?p])
        [?p :block/journal? true]
        [?p :block/journal-day ?d])
      [?h :block/scheduled ?d]
      [?h :block/deadline ?d])
    [(< ?d ?next)]"#;

/// One open task per branch of GH1017_WHERE, plus tasks it must not list.
/// Dates are relative to today, so `:7d-after` puts the window end at +7.
fn gh1017_graph(name: &str, config: Option<&str>) -> Graph {
    let today = tine_core::date::JournalDate::today();
    let day = |offset| today.add_days(offset);
    let stamp = |offset: i64| {
        let d = day(offset);
        let weekday =
            ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][d.to_days().rem_euclid(7) as usize];
        format!("<{:04}-{:02}-{:02} {weekday}>", d.year, d.month, d.day)
    };
    let tasks = format!(
        "- TODO refs a near journal [[{near}]]\n\
         - TODO refs a far journal [[{far}]]\n\
         - TODO refs an ordinary page [[Ordinary]]\n\
         - TODO scheduled soon\n  SCHEDULED: {soon}\n\
         - LATER deadline overdue\n  DEADLINE: {past}\n\
         - TODO scheduled far\n  SCHEDULED: {later}\n\
         - TODO scheduled far but refs a near journal [[{near}]]\n  SCHEDULED: {later}\n\
         - TODO scheduled and deadline soon\n  SCHEDULED: {soon}\n  DEADLINE: {past}\n\
         - DONE closed and scheduled soon\n  SCHEDULED: {soon}\n\
         - NOW doing and scheduled soon\n  SCHEDULED: {soon}\n",
        near = day(2).title(),
        far = day(20).title(),
        soon = stamp(1),
        past = stamp(-5),
        later = stamp(30),
    );
    let past_journal = format!("journals/{}.md", day(-3).file_stem());
    let later_journal = format!("journals/{}.md", day(10).file_stem());
    let mut pages = vec![
        ("pages/Tasks.md", tasks.as_str()),
        (
            past_journal.as_str(),
            "- TODO on a past journal\n- DONE closed on a past journal\n",
        ),
        (later_journal.as_str(), "- TODO on a later journal\n"),
    ];
    if let Some(config) = config {
        pages.push(("logseq/config.edn", config));
    }
    open(name, &pages)
}

fn gh1017_expected() -> Vec<String> {
    let near = tine_core::date::JournalDate::today().add_days(2).title();
    let mut expected = vec![
        format!("TODO refs a near journal [[{near}]]"),
        "TODO scheduled soon".to_owned(),
        "LATER deadline overdue".to_owned(),
        format!("TODO scheduled far but refs a near journal [[{near}]]"),
        "TODO scheduled and deadline soon".to_owned(),
        "TODO on a past journal".to_owned(),
    ];
    expected.sort();
    expected
}

#[test]
fn gh1017_reporter_or_join_over_dates_lists_each_branch_once() {
    let _integration_guard = crate::integration_lock();
    let graph = gh1017_graph("gh1017", None);
    let query = format!(
        "{{:title \"Incoming and overdue\" :query [:find (pull ?h [*]) :in $ ?next :where {GH1017_WHERE}] :inputs [:7d-after] :collapsed? false}}"
    );
    let (raws, result) = answer(&graph, &query);
    assert!(result.supported, "{:?}", result.ignored);
    assert!(result.ignored.is_empty(), "{:?}", result.ignored);
    assert_eq!(raws, gh1017_expected());

    // The same three branches under `or` with each in its own order.
    let reordered = GH1017_WHERE.replace(
        "(or-join [?h ?d]\n      (and\n        (or [?h :block/page ?p] [?h :block/ref-pages ?p])\n        [?p :block/journal? true]\n        [?p :block/journal-day ?d])\n      [?h :block/scheduled ?d]\n      [?h :block/deadline ?d])",
        "(or-join [?d ?h] [?h :block/deadline ?d] [?h :block/scheduled ?d] (and (or [?h :block/ref-pages ?p] [?h :block/page ?p]) [?p :block/journal-day ?d] [?p :block/journal? true]))",
    );
    assert_ne!(reordered, GH1017_WHERE);
    let (raws, result) = answer(
        &graph,
        &format!(
            "{{:query [:find (pull ?h [*]) :in $ ?next :where {reordered}] :inputs [:7d-after]}}"
        ),
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert_eq!(raws, gh1017_expected());

    // Today's two-branch `or` still runs.
    let (raws, result) = answer(
        &graph,
        r#"{:query [:find (pull ?h [*]) :in $ ?next :where [?h :block/marker ?m]
            [(contains? #{"LATER" "TODO"} ?m)]
            (or [?h :block/scheduled ?d] [?h :block/deadline ?d]) [(< ?d ?next)]]
            :inputs [:7d-after]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert_eq!(
        raws,
        sorted(&[
            "LATER deadline overdue",
            "TODO scheduled and deadline soon",
            "TODO scheduled soon"
        ])
    );

    // Three date branches under plain `or` (all bind only ?h and ?d).
    let (raws, result) = answer(
        &graph,
        r#"{:query [:find (pull ?h [*]) :in $ ?next :where [?h :block/marker "TODO"]
            (or [?h :block/scheduled ?d] [?h :block/deadline ?d] [?h :block/scheduled ?d])
            [(< ?d ?next)]] :inputs [:7d-after]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert_eq!(
        raws,
        sorted(&["TODO scheduled and deadline soon", "TODO scheduled soon"])
    );

    // Outside the model: still refused whole.
    for unsupported in [
        // The join variables are not the block and the date.
        r#"(or-join [?h] [?h :block/scheduled ?d] [?h :block/deadline ?d]) [(< ?d ?next)]"#,
        // A plain `or` whose branches bind different variables: DataScript
        // rejects it ("All clauses in 'or' must use same set of free vars").
        r#"(or [?h :block/scheduled ?d] (and [?h :block/page ?p] [?p :block/journal-day ?d])) [(< ?d ?next)]"#,
        // The page is joined outside the group.
        r#"(or (and [?h :block/page ?p] [?p :block/journal-day ?d]) (and [?h :block/ref-pages ?p] [?p :block/journal-day ?d])) [?p :block/name "x"] [(< ?d ?next)]"#,
        // A branch binds another date variable.
        r#"(or-join [?h ?d] [?h :block/scheduled ?d] [?h :block/deadline ?e]) [(< ?d ?next)]"#,
        // A branch is not a date shape.
        r#"(or-join [?h ?d] [?h :block/scheduled ?d] [?h :block/marker ?d]) [(< ?d ?next)]"#,
    ] {
        let (raws, result) = answer(
            &graph,
            &format!("{{:query [:find (pull ?h [*]) :in $ ?next :where {unsupported}] :inputs [:7d-after]}}"),
        );
        assert!(!result.supported, "{unsupported}");
        assert!(raws.is_empty(), "{unsupported}");
    }

    // The reporter's comment-1 workaround on the same data: it lists the
    // scheduled and deadline tasks, so "all empty" was not a lowering bug.
    for (attribute, expected) in [
        (
            "scheduled",
            sorted(&["TODO scheduled soon", "TODO scheduled and deadline soon"]),
        ),
        (
            "deadline",
            sorted(&["LATER deadline overdue", "TODO scheduled and deadline soon"]),
        ),
    ] {
        let (raws, result) = answer(
            &graph,
            &format!(
                "{{:query [:find (pull ?h [*]) :in $ ?next :where [?h :block/marker ?m] [(contains? #{{\"LATER\" \"TODO\"}} ?m)] [?h :block/{attribute} ?d] [(< ?d ?next)]] :inputs [:7d-after] :collapsed? false}}"
            ),
        );
        assert!(result.supported, "{:?}", result.ignored);
        assert_eq!(raws, expected, "{attribute}");
    }
}

/// GH #1017: a date disjunction runs only when each alternative is a scheduled
/// date, a deadline, the journal day of the block's page or of a journal it
/// references, on the one date variable, and that variable is used afterwards
/// only by predicates. DataScript refuses every shape below or answers it
/// differently, so Tine refuses it whole instead of listing rows.
#[test]
fn gh1017_date_disjunctions_outside_the_closed_list_are_refused() {
    let _integration_guard = crate::integration_lock();
    let graph = gh1017_graph("gh1017-refused", None);
    let journal =
        "(and (or [?h :block/page ?p] [?h :block/ref-pages ?p]) [?p :block/journal-day ?d])";
    let refused = [
        // An alternative that does not bind the date.
        r#"(or-join [?h ?d] [?h :block/scheduled ?d] [?h :block/marker "TODO"])"#.to_owned(),
        "(or-join [?h ?d] [?h :block/scheduled ?d] (and [?h :block/page ?p]))".to_owned(),
        "(or-join [?h ?d] [?h :block/scheduled ?d] [?h :block/page ?p])".to_owned(),
        "(or-join [?h ?d] [?h :block/scheduled ?d] (and))".to_owned(),
        "(or-join [?h ?d] [?h :block/scheduled ?d] [?h :block/deadline])".to_owned(),
        "(or-join [?h ?d] [?h :block/scheduled ?d] [?h :block/deadline _])".to_owned(),
        "(or-join [?h ?d] [?h :block/scheduled ?d] [?h :block/deadline 20261201])".to_owned(),
        "(or-join [?h ?d] [?h :block/scheduled ?d] (not [?h :block/deadline ?d]))".to_owned(),
        // A scheduled arm whose date is private to an inner `or-join`.
        "(or-join [?h ?d] (or-join [?h] [?h :block/scheduled ?d]) [?h :block/deadline ?d])"
            .to_owned(),
        "(or (or-join [?h] [?h :block/scheduled ?d]) [?h :block/deadline ?d])".to_owned(),
        "(or-join [?h ?d] (and (or (or-join [?h] [?h :block/page ?p]) [?h :block/ref-pages ?p]) [?p :block/journal-day ?d]) [?h :block/deadline ?d])".to_owned(),
        // A nested `and`.
        "(or-join [?h ?d] (and (and [?h :block/page ?p] [?p :block/journal-day ?d])) [?h :block/deadline ?d])".to_owned(),
        // Join vectors that are not the block and the date as plain variables.
        format!("(or-join [[?h] ?d] {journal} [?h :block/scheduled ?d])"),
        format!("(or-join [[?d] ?h] {journal} [?h :block/scheduled ?d])"),
        format!("(or-join [?h [?d]] {journal} [?h :block/scheduled ?d])"),
        // A predicate inside an arm.
        "(or-join [?h ?d] (and [?h :block/scheduled ?d] [(< ?d ?next)]) [?h :block/deadline ?d])"
            .to_owned(),
        // A scheduled arm with a second constraint (a referenced journal).
        "(or-join [?h ?d] (and [?h :block/scheduled ?d] [?h :block/ref-pages ?p] [?p :block/journal? true]) [?h :block/deadline ?d])".to_owned(),
    ];
    for group in &refused {
        for after in ["", "[(< ?d ?next)]"] {
            let (raws, result) = answer(
                &graph,
                &format!("{{:query [:find (pull ?h [*]) :in $ ?next :where [?h :block/marker \"TODO\"] {group} {after}] :inputs [:7d-after]}}"),
            );
            assert!(!result.supported, "{group} {after}: {raws:?}");
            assert!(raws.is_empty(), "{group} {after}");
            assert!(!result.ignored.is_empty(), "{group} {after}");
        }
    }

    // A predicate on the date before the group: DataScript reports the date
    // unbound. This includes the two-arm `or` that Tine ran before.
    for group in [
        format!("(or-join [?h ?d] {journal} [?h :block/scheduled ?d] [?h :block/deadline ?d])"),
        "(or [?h :block/scheduled ?d] [?h :block/deadline ?d] [?h :block/scheduled ?d])".to_owned(),
        "(or [?h :block/scheduled ?d] [?h :block/deadline ?d])".to_owned(),
    ] {
        let (raws, result) = answer(
            &graph,
            &format!("{{:query [:find (pull ?h [*]) :in $ ?next :where [?h :block/marker \"TODO\"] [(< ?d ?next)] {group}] :inputs [:7d-after]}}"),
        );
        assert!(!result.supported, "{group}: {raws:?}");
        assert!(raws.is_empty(), "{group}");
    }

    // A numeric `:in` binding of the block variable: DataScript lists only
    // that entity (none here), so the group does not run on every task.
    for group in [
        format!("(or-join [?h ?d] {journal} [?h :block/scheduled ?d] [?h :block/deadline ?d])"),
        "(or [?h :block/scheduled ?d] [?h :block/deadline ?d])".to_owned(),
    ] {
        let (raws, result) = answer(
            &graph,
            &format!("{{:query [:find (pull ?h [*]) :in $ ?h ?next :where [?h :block/marker \"TODO\"] {group} [(< ?d ?next)]] :inputs [123456 :7d-after]}}"),
        );
        assert!(!result.supported, "{group}: {raws:?}");
        assert!(raws.is_empty(), "{group}");
    }
}

/// The reporter's surface: the entry read from `logseq/config.edn`
/// `:default-queries :journals`, its macro body run as the journal does.
#[test]
fn gh1017_config_default_query_runs_through_the_journal_config_path() {
    let _integration_guard = crate::integration_lock();
    let config = format!(
        "{{:default-queries\n {{:journals\n  [{{:title \"Incoming and overdue\"\n    :query [:find (pull ?h [*])\n            :in $ ?next\n            :where\n            {GH1017_WHERE}]\n    :inputs [:7d-after]\n    :collapsed? false}}]}}}}"
    );
    let graph = gh1017_graph("gh1017-config", Some(&config));
    let state = graph._store.config();
    let [entry] = &state.config.default_journal_queries[..] else {
        panic!("{:?}", state.config.default_journal_queries)
    };
    assert_eq!(entry.title.as_deref(), Some("Incoming and overdue"));
    assert!(entry.error.is_none(), "{:?}", entry.error);
    let source = entry.body.strip_prefix("query ").expect("macro body");
    let (raws, result) = answer(&graph, source);
    assert!(result.supported, "{:?}", result.ignored);
    assert_eq!(raws, gh1017_expected());
}
