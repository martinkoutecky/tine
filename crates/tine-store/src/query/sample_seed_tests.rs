//! Audit #6: OG `query` (`query_dsl.cljs:583-589`) takes `(take n (shuffle rows))`
//! BEFORE the sort, so a sample is a random subset of the filtered result, not
//! its first n after sorting. The seed is the only randomness in query
//! execution; these tests pin it (a lib test because the pin is crate-private:
//! the public surface may not grow, `tests/shallow_ratchet.rs`).

use std::collections::BTreeSet;

use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, QueryInput};

use crate::{IrAnswer, IrRequest, Store, WholeGraph};

fn fixture() -> (tempfile::TempDir, WholeGraph) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("pages")).unwrap();
    std::fs::write(
        dir.path().join("pages/Work.md"),
        "- [#A] urgent\n- [#B] mid\n- [#C] low\n- plain priority text\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    (dir, graph)
}

fn priorities(graph: &WholeGraph, q: &str) -> Vec<char> {
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
        .filter_map(|raw| raw.split("[#").nth(1).and_then(|rest| rest.chars().next()))
        .collect()
}

#[test]
fn sample_is_a_random_subset_taken_before_the_sort() {
    let q = "(and (page \"Work\") (priority A B C) (sample 2) (sort-by priority))";
    let mut subsets = BTreeSet::new();
    for seed in 0..24u64 {
        // A fresh graph per seed: results are memoized per query text.
        let (_dir, graph) = fixture();
        let _pin = super::exec::pin_sample_seed(seed);
        let got = priorities(&graph, q);
        assert_eq!(got.len(), 2, "seed {seed}: {got:?}");
        assert!(
            got[0] > got[1],
            "a sample is sorted afterwards (desc): {got:?}"
        );
        let again = {
            let (_dir, graph) = fixture();
            priorities(&graph, q)
        };
        assert_eq!(got, again, "the same seed picks the same subset");
        subsets.insert(got);
    }
    // Sorting first and truncating (the old behaviour) always gave [C, B].
    assert!(subsets.len() > 1, "every seed gave one subset: {subsets:?}");
    // A sample larger than the result changes nothing.
    let (_dir, graph) = fixture();
    assert_eq!(
        priorities(
            &graph,
            "(and (page \"Work\") (priority A B C) (sample 99) (sort-by priority))"
        ),
        ['C', 'B', 'A']
    );
}
