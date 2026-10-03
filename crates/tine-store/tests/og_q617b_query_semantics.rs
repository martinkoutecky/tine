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
//!   that only occurs in a property line (#624). Tine keeps its documented
//!   case-insensitive superset (SPEC); only the property lines are new.
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
    for q in ["(priority [A])", "(priority [a])", "(priority [\"A\"])"] {
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
            &["TODO DOING", "#{TODO DOING}", "(TODO DOING)", "[:todo :doing]"],
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
        assert!(want.len() >= 2, "({form} {}) should match: {want:?}", spellings[0]);
        for spelling in spellings {
            let q = format!("({form} {spelling})");
            assert_eq!(set(&graph, &q), want, "{q}");
            // The rest of the enclosing form must still parse after the collection.
            let q = format!("(and ({form} {spelling}) (not \"closed\"))");
            assert_eq!(
                set(&graph, &q),
                want.iter().filter(|b| !b.contains("closed")).cloned().collect::<BTreeSet<_>>(),
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
            query.diagnostics.iter().any(|d| d.kind == tine_core::query::ir::DiagnosticKind::Syntax),
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
    let urgent: BTreeSet<String> = work.iter().filter(|b| b.contains("urgent")).cloned().collect();
    assert_eq!(urgent.len(), 1, "{work:?}");
    // One operand: the rows it matches go.
    let minus_a = set(&graph, "(and (page \"Work\") (not (priority A)))");
    assert_eq!(minus_a, work.difference(&urgent).cloned().collect::<BTreeSet<_>>());
    // Two operands that no single row satisfies together drop nothing, although
    // each alone would drop rows (the old reader dropped everything: a syntax error).
    assert_eq!(set(&graph, "(and (page \"Work\") (not (priority A) (priority B)))"), work);
    assert_eq!(set(&graph, "(and (page \"Work\") (not (task TODO) (priority A)))"), work);
    // Operands that one row satisfies together drop exactly that row.
    assert_eq!(set(&graph, "(and (page \"Work\") (not (priority A) \"urgent\"))"), minus_a);
    // Variadic `not` also works at the top of the form and with a directive.
    assert!(!set(&graph, "(not (priority A) \"urgent\" (sort-by priority))").is_empty());
}
