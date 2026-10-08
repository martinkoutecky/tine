//! Discussions #617/#624/#619: what a simple query RETURNS must equal OG.
//!
//! OG provenance (read-only checkout, `src/main/frontend/db/query_dsl.cljs`):
//! * `build-task` (:279), `build-priority` (:289), `build-page-tags` (:311):
//!   `(if (coll? (first (rest e))) (first (rest e)) (rest e))` -- a leading
//!   Clojure vector supplies the whole list, otherwise the names are variadic.
//!   Tine's tokenizer used to read `[A B]` as two junk words (`[A`, `B]`), so
//!   `(priority [A])` matched nothing (#624).
//! * `build-block-content` (:373) + `rules.cljc:114` `block-content`:
//!   `clojure.string/includes?` over the block's RAW `:block/content`, which
//!   includes `key:: value` property lines. A bare string therefore finds text
//!   that only occurs in a property line (#624). Martin D4 (2026-10-04)
//!   also restored exact case and accents over the complete raw content.
//! * `(and [[P]] (not (task TODO)))` (#619 4a): a bare page ref is OG's
//!   `:page-ref` rule over `:block/path-refs`, which includes the block's own
//!   page, so blocks of page P other than TODO tasks are returned.
//!
//! Every query goes through the path a `{{query}}` block takes
//! (`parse_query_input(.., MacroQuery, ..)` then `query_ir(Run)`).

use std::collections::BTreeSet;

use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, QueryInput};
use tine_store::{IrAnswer, IrRequest, Store, WholeGraph};

/// Raw text of every block a macro query returns, in result order.
fn raws(graph: &WholeGraph, q: &str) -> Vec<String> {
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
        .flat_map(|g| g.blocks.into_iter().map(|b| b.raw))
        .collect()
}

/// Page names a page-anchored macro query returns.
fn page_names(graph: &WholeGraph, q: &str) -> BTreeSet<String> {
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
    let QueryRows::Page { pages } = result.rows else {
        panic!("`{q}` is page-anchored");
    };
    pages.into_iter().map(|p| format!("{p:?}")).collect()
}

fn set(graph: &WholeGraph, q: &str) -> BTreeSet<String> {
    raws(graph, q).into_iter().collect()
}

fn fixture() -> (tempfile::TempDir, WholeGraph) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("journals")).unwrap();
    let w = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    w(
        "pages/BugQueries.md",
        "- {{query (and [[BugQueries]] (not (task TODO)))}}\n- Items\n  - TODO One\n- TODO Parent\n  - DONE Two\n- [[BugQueries]]: Do something\n",
    );
    w(
        "pages/Work.md",
        "tags:: alpha, beta\n\n- [#A] urgent\n- [#B] mid\n- [#C] low\n- plain priority text\n- TODO open\n- DOING busy\n- DONE closed\n- Note\n  foo:: quuxvalue\n  text Hello World\n",
    );
    w("pages/Other.md", "tags:: gamma\n\n- [#A] other urgent\n");
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    (dir, graph)
}

#[test]
fn priority_accepts_the_vector_form_like_og() {
    let (_dir, graph) = fixture();
    let a_only = set(&graph, "(priority A)");
    assert_eq!(a_only.len(), 2, "{a_only:?}");
    for q in [
        "(priority [A])",
        "(priority [a])",
        "(priority [\"A\"])",
        "(priority [#A])",
    ] {
        assert_eq!(set(&graph, q), a_only, "{q}");
    }
    let ab = set(&graph, "(priority A B)");
    assert_eq!(ab.len(), 3, "{ab:?}");
    for q in [
        "(priority [A B])",
        "(priority [A, B])",
        "(priority [A B] C)",
    ] {
        assert_eq!(set(&graph, q), ab, "{q}");
    }
}

#[test]
fn task_accepts_the_vector_form_like_og() {
    let (_dir, graph) = fixture();
    let todo = set(&graph, "(task TODO)");
    assert!(todo.iter().any(|b| b.contains("open")), "{todo:?}");
    assert_eq!(set(&graph, "(task [TODO])"), todo);
    let two = set(&graph, "(task TODO DOING)");
    assert_eq!(set(&graph, "(task [TODO DOING])"), two);
    assert!(two.iter().any(|b| b.contains("busy")), "{two:?}");
    assert_eq!(set(&graph, "(todo [todo doing])"), two);
}

/// GH #422: Tine intentionally differs from OG Logseq's dropped bare task filter.
#[test]
fn bare_task_intentionally_differs_from_og_in_markdown_and_org() {
    let open = [
        "TODO",
        "DOING",
        "NOW",
        "LATER",
        "WAITING",
        "WAIT",
        "STARTED",
        "IN-PROGRESS",
    ];
    let finished = ["DONE", "CANCELED", "CANCELLED"];
    for (extension, bullet) in [("md", "-"), ("org", "*")] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        let mut text = String::new();
        for marker in open.iter().chain(finished.iter()) {
            text.push_str(&format!("{bullet} {marker} work\n"));
        }
        text.push_str(&format!(
            "{bullet} ordinary prose\n{bullet} TODO: not a task\n"
        ));
        std::fs::write(dir.path().join(format!("pages/Tasks.{extension}")), text).unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let graph = store.whole_graph().unwrap();
        let expected = open.iter().map(|m| format!("{m} work")).collect();
        assert_eq!(set(&graph, "(task)"), expected, "{extension}");
        assert_eq!(set(&graph, "(todo)"), expected, "{extension} alias");
        for marker in open.iter().chain(finished.iter()) {
            assert_eq!(
                set(&graph, &format!("(task {marker})")),
                BTreeSet::from([format!("{marker} work")]),
                "{extension}: {marker}"
            );
        }
    }
}

#[test]
fn page_tags_accepts_the_vector_form_like_og() {
    let (_dir, graph) = fixture();
    let work = page_names(&graph, "(page-tags alpha)");
    assert_eq!(work.len(), 1, "{work:?}");
    assert_eq!(page_names(&graph, "(page-tags [alpha])"), work);
    let two = page_names(&graph, "(page-tags alpha gamma)");
    assert_eq!(two.len(), 2, "{two:?}");
    assert_eq!(page_names(&graph, "(page-tags [alpha gamma])"), two);
}

#[test]
fn bare_string_search_sees_property_lines_like_og_raw_content() {
    let (_dir, graph) = fixture();
    // `quuxvalue` occurs only in the `foo:: quuxvalue` line.
    for q in [
        "\"quuxvalue\"",
        "\"foo:: quuxvalue\"",
        "(and \"quuxvalue\" \"Hello\")",
    ] {
        let hits = raws(&graph, q);
        assert_eq!(hits.len(), 1, "{q}: {hits:?}");
        assert!(hits[0].contains("quuxvalue"), "{q}");
    }
    // A string in neither the body nor a property still matches nothing.
    assert!(raws(&graph, "\"absent-needle\"").is_empty());
}

#[test]
fn page_ref_and_not_task_returns_the_pages_other_blocks() {
    let (_dir, graph) = fixture();
    // #619 4a. OG path-refs include the block's own page, so every block of
    // BugQueries that is not a TODO task qualifies (the host block is dropped
    // by the result layer, not the engine).
    let hits = set(&graph, "(and [[BugQueries]] (not (task TODO)))");
    assert!(hits.iter().any(|b| b.contains("Items")), "{hits:?}");
    // A TODO parent is excluded, so its DONE child is a top-level result;
    // a matching parent would absorb its children (`filter-top-level-blocks`).
    assert!(hits.iter().any(|b| b.contains("DONE Two")), "{hits:?}");
    assert!(!hits.iter().any(|b| b.contains("TODO Parent")), "{hits:?}");
    assert!(!hits.iter().any(|b| b.contains("TODO One")), "{hits:?}");
}

/// Audit #3: OG reads the FIRST argument of `task`/`todo`/`priority`/`page-tags`
/// as the whole collection when it is any Clojure collection (`coll?`: vector,
/// set or list); otherwise the names are variadic, as symbols, keywords or
/// quoted strings (`query_dsl.cljs:279-320`). Every spelling of one list answers
/// the same, for every form that takes it.
#[test]
fn every_collection_spelling_answers_the_same_for_task_priority_and_page_tags() {
    let (_dir, graph) = fixture();
    // (form, one-argument spellings of the same list, expected is non-empty)
    let blocks: [(&str, &[&str]); 3] = [
        (
            "task",
            &[
                "TODO DOING",
                "[TODO DOING]",
                "[TODO, DOING]",
                "#{TODO DOING}",
                "(TODO DOING)",
                "[\"TODO\" \"DOING\"]",
                "#{\"todo\" doing}",
                "\"TODO\" \"DOING\"",
                ":todo :doing",
                "[:todo :doing]",
                "#{:todo, :doing}",
                "[TODO DOING] ignored",
                "#{TODO DOING} (never-read)",
            ],
        ),
        (
            "todo",
            &[
                "TODO DOING",
                "#{TODO DOING}",
                "(TODO DOING)",
                "[:todo :doing]",
            ],
        ),
        (
            "priority",
            &[
                "A B",
                "[A B]",
                "#{A B}",
                "(A B)",
                "(a, b)",
                "\"A\" \"B\"",
                ":a :b",
                "#{:a :b} C",
            ],
        ),
    ];
    for (form, spellings) in blocks {
        let want = set(&graph, &format!("({form} {})", spellings[0]));
        assert!(
            want.len() >= 2,
            "({form} {}) should match: {want:?}",
            spellings[0]
        );
        for spelling in spellings {
            let q = format!("({form} {spelling})");
            assert_eq!(set(&graph, &q), want, "{q}");
            // The rest of the enclosing form must still parse after the collection.
            let q = format!("(and ({form} {spelling}) (not \"closed\"))");
            assert_eq!(
                set(&graph, &q),
                want.iter()
                    .filter(|b| !b.contains("closed"))
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                "{q}"
            );
        }
    }
    let want = page_names(&graph, "(page-tags alpha gamma)");
    assert_eq!(want.len(), 2, "{want:?}");
    for spelling in [
        "[alpha gamma]",
        "[alpha, gamma]",
        "#{alpha gamma}",
        "(alpha gamma)",
        "[\"alpha\" \"gamma\"]",
        "[ [[alpha]] [[gamma]] ]",
        "#{#alpha #gamma}",
        ":alpha :gamma",
        "\"alpha\" \"gamma\"",
        "#{alpha gamma} beta",
    ] {
        let q = format!("(page-tags {spelling})");
        assert_eq!(page_names(&graph, &q), want, "{q}");
    }
}

/// A collection that Logseq's reader cannot turn into names makes the whole
/// query fail there; Tine reports it instead of matching a guess.
#[test]
fn a_nested_or_unclosed_collection_is_a_query_error_not_a_guess() {
    for q in ["(task [TODO [DOING]])", "(task #{TODO", "(priority (A ])"] {
        let (query, _view) = parse_query_input(
            q,
            QueryInput::MacroQuery,
            tine_core::date::JournalDate::today(),
            Registry::none(),
        );
        assert!(
            query
                .diagnostics
                .iter()
                .any(|d| d.kind == tine_core::query::ir::DiagnosticKind::Syntax),
            "{q}: {:?}",
            query.diagnostics
        );
    }
}

/// Audit #5: OG `not` is variadic (`query_dsl.cljs:128-142`): `(not a b)` is
/// datalog `(not a b)`, which drops a row only when ALL its clauses hold.
#[test]
fn not_negates_the_conjunction_of_all_its_operands() {
    let (_dir, graph) = fixture();
    let work = set(&graph, "(page \"Work\")");
    let urgent: BTreeSet<String> = work
        .iter()
        .filter(|b| b.contains("urgent"))
        .cloned()
        .collect();
    assert_eq!(urgent.len(), 1, "{work:?}");
    // One operand: the rows it matches go.
    let minus_a = set(&graph, "(and (page \"Work\") (not (priority A)))");
    assert_eq!(
        minus_a,
        work.difference(&urgent).cloned().collect::<BTreeSet<_>>()
    );
    // Two operands that no single row satisfies together drop nothing, although
    // each alone would drop rows (the old reader dropped everything: a syntax error).
    assert_eq!(
        set(
            &graph,
            "(and (page \"Work\") (not (priority A) (priority B)))"
        ),
        work
    );
    assert_eq!(
        set(
            &graph,
            "(and (page \"Work\") (not (task TODO) (priority A)))"
        ),
        work
    );
    // Operands that one row satisfies together drop exactly that row.
    assert_eq!(
        set(
            &graph,
            "(and (page \"Work\") (not (priority A) \"urgent\"))"
        ),
        minus_a
    );
    // Variadic `not` also works at the top of the form and with a directive.
    assert!(!set(&graph, "(not (priority A) \"urgent\" (sort-by priority))").is_empty());
}

fn priorities(graph: &WholeGraph, q: &str) -> Vec<char> {
    raws(graph, q)
        .iter()
        .filter_map(|raw| raw.split("[#").nth(1).and_then(|rest| rest.chars().next()))
        .collect()
}

/// Audit #6: OG `build-sort-by` (`query_dsl.cljs:335-348`) is `:desc` unless the
/// keyword is exactly `asc`.
#[test]
fn sort_by_defaults_to_descending_like_og() {
    let (_dir, graph) = fixture();
    let base = "(and (page \"Work\") (priority A B C)";
    assert_eq!(
        priorities(&graph, &format!("{base} (sort-by priority))")),
        ['C', 'B', 'A']
    );
    assert_eq!(
        priorities(&graph, &format!("{base} (sort-by priority desc))")),
        ['C', 'B', 'A']
    );
    assert_eq!(
        priorities(&graph, &format!("{base} (sort-by priority :desc))")),
        ['C', 'B', 'A']
    );
    assert_eq!(
        priorities(&graph, &format!("{base} (sort-by priority asc))")),
        ['A', 'B', 'C']
    );
    assert_eq!(
        priorities(&graph, &format!("{base} (sort-by priority :asc))")),
        ['A', 'B', 'C']
    );
    // Any other word is not `:asc`, so OG sorts descending.
    assert_eq!(
        priorities(&graph, &format!("{base} (sort-by priority upward))")),
        ['C', 'B', 'A']
    );
}

/// Audit #6: a query made only of directives is OG's nil query: it runs nothing.
#[test]
fn a_query_of_only_directives_returns_nothing_like_og() {
    let (_dir, graph) = fixture();
    for q in [
        "(sort-by priority)",
        "(sample 3)",
        "(and (sort-by priority))",
        "(and (sample 2) (sort-by priority desc))",
    ] {
        assert!(page_names(&graph, q).is_empty(), "{q}");
    }
    // A directive beside a real clause still shapes it.
    assert!(!raws(&graph, "(and (priority A) (sort-by priority))").is_empty());
}

const JAN_1_2024: i64 = 1_704_067_200_000;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// UTC midnight of the local civil `today`: the anchor of every `d`/`h`/`n`
/// offset (OG `->timestamp` adds to `(t/today)`, not to the current instant).
fn midnight_today() -> i64 {
    tine_core::date::JournalDate::today().to_days() * DAY
}

fn timestamp_graph() -> (tempfile::TempDir, WholeGraph) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("journals")).unwrap();
    let midnight = midnight_today();
    let now = now_ms();
    let blocks = [
        ("first-instant", format!("created-at:: {JAN_1_2024}")),
        ("noon", format!("created-at:: {}", JAN_1_2024 + 12 * HOUR)),
        (
            "second-midnight",
            format!("created-at:: {}", JAN_1_2024 + DAY),
        ),
        ("snake", format!("created_at:: {}", JAN_1_2024 + HOUR)),
        (
            "modified",
            format!("last-modified-at:: {}", JAN_1_2024 + 2 * HOUR),
        ),
        ("recent", format!("created-at:: {}", now - 1000)),
        ("a-month-ago", format!("created-at:: {}", now - 30 * DAY)),
        ("long-ago", format!("created-at:: {}", now - 2000 * DAY)),
        ("tomorrow-ish", format!("created-at:: {}", now + DAY)),
        (
            "midnight-plus-30h",
            format!("created-at:: {}", midnight + 30 * HOUR),
        ),
        (
            "midnight-plus-10h",
            format!("created-at:: {}", midnight + 10 * HOUR),
        ),
        (
            "midnight-plus-90n",
            format!("created-at:: {}", midnight + 90 * 60_000),
        ),
        ("not-a-number", "created-at:: yesterday".to_string()),
        ("no-stamp", "other:: 1".to_string()),
    ];
    let mut text = String::new();
    for (label, prop) in blocks {
        text.push_str(&format!("- {label}\n  {prop}\n"));
    }
    std::fs::write(dir.path().join("pages/Stamped.md"), text).unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    (dir, graph)
}

fn labels(graph: &WholeGraph, q: &str) -> BTreeSet<String> {
    raws(graph, q)
        .into_iter()
        .map(|raw| {
            raw.lines()
                .next()
                .unwrap_or("")
                .trim_start_matches("- ")
                .to_string()
        })
        .collect()
}

fn want(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// Audit #4: `(between created-at START END)` (OG `build-between-three-arg`,
/// query_dsl.cljs:214-229) is a range over the block's timestamp property --
/// lower bound inclusive, upper exclusive, the pair sorted. Before the fix the
/// parser read `created-at` as the FIRST BOUND of the journal-day form and the
/// query matched nothing.
#[test]
fn between_created_at_is_a_half_open_range_over_the_timestamp_property() {
    let (_dir, graph) = timestamp_graph();
    let jan_1_2 = "[[Jan 1st, 2024]] [[Jan 2nd, 2024]]";
    let expected = want(&["first-instant", "noon", "snake"]);
    for field in ["created-at", "created_at", "CREATED-AT", "Created_At"] {
        assert_eq!(
            labels(&graph, &format!("(between {field} {jan_1_2})")),
            expected,
            "{field}: lower inclusive, upper exclusive (second-midnight is out)"
        );
    }
    assert_eq!(
        labels(
            &graph,
            "(between created-at [[Jan 2nd, 2024]] [[Jan 1st, 2024]])"
        ),
        expected,
        "OG sorts the two bounds"
    );
    assert_eq!(
        labels(&graph, &format!("(between last-modified-at {jan_1_2})")),
        want(&["modified"]),
        "last-modified-at reads its own property, never created-at's"
    );
    assert_eq!(
        labels(&graph, &format!("(between last_modified_at {jan_1_2})")),
        want(&["modified"])
    );
}

#[test]
fn between_timestamp_bounds_read_now_and_h_n_offsets_like_og_to_timestamp() {
    let (_dir, graph) = timestamp_graph();
    // The wall clock decides where `now` falls relative to the midnight-anchored
    // blocks, so each assertion looks only at the family it is about.
    let only = |q: &str, prefix: &str| -> BTreeSet<String> {
        labels(&graph, q)
            .into_iter()
            .filter(|label| label.starts_with(prefix))
            .collect()
    };
    let clock = |q: &str| {
        let mut found = only(q, "recent");
        found.extend(only(q, "a-month"));
        found.extend(only(q, "long"));
        found.extend(only(q, "tomorrow"));
        found
    };
    // `now` is the current instant; `-1000d` is 1000 days before UTC midnight.
    assert_eq!(
        clock("(between created-at -1000d now)"),
        want(&["recent", "a-month-ago"]),
        "now is the instant, not the day"
    );
    // `+24h`/`+48h` are hours past UTC MIDNIGHT of today (not past now).
    assert_eq!(
        only("(between created-at +24h +48h)", "midnight"),
        want(&["midnight-plus-30h"])
    );
    assert_eq!(
        only("(between created-at +1d +2d)", "midnight"),
        want(&["midnight-plus-30h"]),
        "24h is the same bound as 1d"
    );
    assert_eq!(
        only("(between created-at +60n +120n)", "midnight"),
        want(&["midnight-plus-90n"])
    );
    assert_eq!(
        only("(between created-at +8h +12h)", "midnight"),
        want(&["midnight-plus-10h"])
    );
}

#[test]
fn between_timestamp_with_an_unresolvable_or_missing_bound_is_invalid() {
    let (_dir, graph) = timestamp_graph();
    for q in [
        "(between created-at nonsense today)",
        "(between created-at today)",
        "(between created-at)",
        "(between created-at -99999999999999999999h today)",
    ] {
        let (query, _) = parse_query_input(
            q,
            QueryInput::MacroQuery,
            tine_core::date::JournalDate::today(),
            Registry::none(),
        );
        assert!(
            query.is_invalid(),
            "{q}: OG builds no clause unless both bounds resolve"
        );
        assert!(raws(&graph, q).is_empty(), "{q}");
    }
}

/// Audit #8. OG's simple-query `(namespace x)` is the IMMEDIATE-parent rule
/// (`rules.cljc:124-127`): `x/a` matches and `x/a/b` does not. The recursive rule
/// (`rules.cljc:7-12`) belongs to advanced queries and must keep matching
/// descendants.
#[test]
fn simple_namespace_is_immediate_children_only_while_advanced_stays_recursive() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("journals")).unwrap();
    let w = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    w("pages/Project%2FAlpha.md", "- child alpha\n");
    w("pages/Project%2FAlpha%2FBeta.md", "- grandchild beta\n");
    w("pages/Project%2FGamma.md", "- child gamma\n");
    w("pages/Other.md", "- unrelated\n");
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();

    // `(namespace x)` alone is page-anchored; a block-anchored conjunction
    // (`(and (task ..) ..)`) reads the same relation through the block's page.
    let pages = |q: &str| -> Vec<String> {
        let mut v: Vec<String> = page_names(&graph, q)
            .into_iter()
            .map(|debug| {
                debug
                    .split('"')
                    .nth(1)
                    .unwrap_or(debug.as_str())
                    .to_string()
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        pages("(namespace Project)"),
        ["pages/Project%2FAlpha.md", "pages/Project%2FGamma.md"],
        "immediate children only; the grandchild is not a direct child"
    );
    assert_eq!(
        pages("(namespace project/alpha)"),
        ["pages/Project%2FAlpha%2FBeta.md"],
        "the parent is the whole prefix, compared like any page name (case folded)"
    );
    assert_eq!(pages("(namespace Other)"), Vec::<String>::new());

    // The advanced `(namespace ?b "Project")` clause is the recursive rule.
    let (query, view) = parse_query_input(
        r#"[:find (pull ?b [*]) :where (namespace ?b "Project")]"#,
        QueryInput::Advanced,
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
        panic!("advanced block query");
    };
    let mut advanced: Vec<String> = groups
        .into_iter()
        .flat_map(|g| g.blocks.into_iter().map(|b| b.raw.trim().to_string()))
        .collect();
    advanced.sort();
    assert_eq!(
        advanced,
        ["child alpha", "child gamma", "grandchild beta"],
        "advanced queries keep OG's recursive namespace rule"
    );
}

/// Audit #8a: OG renders grouped block results with
/// `(sort-by (comp :block/journal-day first) >)` (`components/block.cljs:3497,
/// 3523, 3552`): with no explicit sort, journal days run NEWEST first and pages
/// that are not journals come after them.
#[test]
fn journal_result_groups_run_newest_first_when_no_sort_is_given() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("journals")).unwrap();
    let w = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    w("journals/2025_12_31.md", "- TODO old\n");
    w("journals/2026_01_01.md", "- TODO new\n");
    w("journals/2024_06_15.md", "- TODO oldest\n");
    w("pages/Alpha.md", "- TODO page a\n");
    w("pages/Zed.md", "- TODO page z\n");
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    assert_eq!(
        raws(&graph, "(task TODO)"),
        [
            "TODO new",
            "TODO old",
            "TODO oldest",
            "TODO page a",
            "TODO page z"
        ]
    );
}
