//! GH #651: a task nested under another task is missing from the Table.
//!
//! Not a defect: Search / List / Table / Board are presentations of ONE result membership, and that
//! membership is OG's top-level-result rule. OG removes a result only when its IMMEDIATE parent is also
//! in the unfiltered result (`frontend/modules/outliner/tree.cljs` `filter-top-level-blocks`, applied to
//! every presentation, the table included, by `frontend/components/query/result.cljs`
//! `get-query-result` unless the advanced query sets `:remove-block-children? false`; OG's own
//! `src/test/.../query/result_test.cljs` pins it for `{:table? true}`). The List shows the removed child
//! because it renders the parent's subtree, not because the child is a result. ADR 0042 records the rule.
//!
//! These tests pin the rule at the entry a `{{query}}` block takes (host `tine.*` properties merged by
//! `parse_query_pair`, then `query_ir(Run)`): every presentation answers the same blocks; a child of a
//! matching parent is absorbed; a child below a NON-matching parent (the reporter's narrowed case, a note
//! parent, a done parent) is its own row; and nesting deeper than one level behaves per immediate parent.
//! There is no fail-before: og already equals OG, so these are pins.

use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::wire_parse::{parse_query_pair, QueryTextDialect};
use tine_store::{IrAnswer, IrRequest, Store, WholeGraph};

/// The reporter's page, with a note parent, a done parent and a grandchild added.
const TASKS: &str = "\
- TODO Parent task A
  SCHEDULED: <2027-06-01 Tue>
\t- TODO Child task A1
\t  SCHEDULED: <2026-10-09 Fri>
- DOING Parent task B
\t- TODO Child task B1
- TODO Parent task C
\t- DOING Child task C1
- TODO Sibling task D
  SCHEDULED: <2026-10-08 Thu>
- Note parent
\t- TODO Child of a note
- DONE Done parent
\t- TODO Child of a done task
\t\t- TODO Grandchild of a done task
";

fn graph() -> (tempfile::TempDir, WholeGraph) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::create_dir(dir.path().join("journals")).unwrap();
    std::fs::write(dir.path().join("pages/Outline.md"), TASKS).unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    (dir, graph)
}

/// First line of every result block, in result order, for the query block `{{query <q>}}` whose own
/// properties are `props` (as the macro reads them).
fn rows(graph: &WholeGraph, q: &str, props: &[(&str, &str)]) -> Vec<String> {
    let props: Vec<(String, String)> = props
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let parsed = parse_query_pair(q, QueryTextDialect::MacroQuery, &props, Registry::none());
    let IrAnswer::Result(result) = graph
        .query_ir(IrRequest::Run {
            query: &parsed.query,
            view: &parsed.view,
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
        .flat_map(|g| {
            g.blocks
                .into_iter()
                .map(|b| b.raw.lines().next().unwrap_or("").to_string())
        })
        .collect()
}

fn sorted(mut rows: Vec<String>) -> Vec<String> {
    rows.sort();
    rows
}

fn names(list: &[&str]) -> Vec<String> {
    sorted(list.iter().map(|s| s.to_string()).collect())
}

#[test]
fn every_presentation_answers_one_membership_and_one_count() {
    let (_dir, graph) = graph();
    // The reporter's query: all four parents match, so A1/B1/C1 sit under a matching parent and are not
    // results. The note's child and the done task's child have a non-matching parent: own rows.
    let expected = names(&[
        "TODO Parent task A",
        "DOING Parent task B",
        "TODO Parent task C",
        "TODO Sibling task D",
        "TODO Child of a note",
        "TODO Child of a done task",
    ]);
    let list = rows(&graph, "(task TODO DOING)", &[]);
    assert_eq!(sorted(list.clone()), expected, "{list:?}");
    for view in ["list", "table", "board", "search"] {
        let shown = rows(
            &graph,
            "(task TODO DOING)",
            &[
                ("tine.view", view),
                ("tine.columns", "state;scheduled;deadline;page"),
                ("tine.sort", "scheduled asc"),
            ],
        );
        assert_eq!(
            sorted(shown.clone()),
            expected,
            "{view} answered a different membership: {shown:?}"
        );
        assert_eq!(
            shown.len(),
            list.len(),
            "{view} counts a different number of results"
        );
    }
}

#[test]
fn a_child_below_a_non_matching_parent_is_its_own_row() {
    let (_dir, graph) = graph();
    let table = [("tine.view", "table")];
    // The reporter's narrowed case: B (DOING) no longer matches, so B1 is a result of its own. C1 is
    // DOING and does not match; A1's parent A still matches and absorbs it.
    assert_eq!(
        sorted(rows(&graph, "(task TODO)", &table)),
        names(&[
            "TODO Parent task A",
            "TODO Child task B1",
            "TODO Parent task C",
            "TODO Sibling task D",
            "TODO Child of a note",
            "TODO Child of a done task",
        ])
    );
    // The mirror: C (TODO) does not match (task DOING), so C1 is a row; B matches but has no DOING child.
    assert_eq!(
        sorted(rows(&graph, "(task DOING)", &table)),
        names(&["DOING Parent task B", "DOING Child task C1"])
    );
}

#[test]
fn suppression_is_per_immediate_parent_at_every_depth() {
    let (_dir, graph) = graph();
    let table = [("tine.view", "table")];
    // The grandchild's parent (a TODO child) matches, so it is absorbed; the child's parent (DONE) does not
    // match, so the child is a row. Removing the middle match surfaces the grandchild instead.
    let shown = rows(&graph, "(task TODO)", &table);
    assert!(
        shown.contains(&"TODO Child of a done task".to_string()),
        "{shown:?}"
    );
    assert!(!shown.iter().any(|r| r.contains("Grandchild")), "{shown:?}");
    let gap = rows(&graph, "(and (task TODO) \"Grandchild\")", &table);
    assert_eq!(gap, vec!["TODO Grandchild of a done task".to_string()]);
}
