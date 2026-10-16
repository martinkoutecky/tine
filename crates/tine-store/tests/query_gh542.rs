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
    let dir = std::env::temp_dir().join(format!("tine-gh542-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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
        [(> ?d ?start)] [(< ?d ?next)]] :inputs [2026-11-01 2026-11-20]}"#,
    );
    assert!(result.supported, "{:?}", result.ignored);
    assert_eq!(raws, ["TODO title [[2026_10_11]]"]);
}
