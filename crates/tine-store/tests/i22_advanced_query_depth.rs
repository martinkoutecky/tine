//! I-22 (og C3 L01): an advanced query's recursive consumers must agree with
//! the admission depth bound. The guard (`query_nesting_within_limit`) reads a
//! quoted `:where "(and (and …))"` as a string at depth 0, but the clause
//! scanner collected the quoted nest as a group and `flatten_single_branch_groups`
//! recursed once per `(and`: a 60 KiB query (under the 64 KiB cap) overflowed
//! the stack and aborted the process on every render of that query — a
//! crash-loop when it sits on an always-shown journal or in :default-queries.

use std::path::PathBuf;

use tine_store::{OpenOptions, QueryDialect, Store};

fn graph(name: &str) -> (PathBuf, Store) {
    let dir = std::env::temp_dir().join(format!("tine-i22-adv-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["pages", "journals"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(dir.join("pages/Tasks.md"), "- TODO one\n- DONE two\n").unwrap();
    let (store, _, _) = Store::open(&dir, OpenOptions::default()).expect("open");
    (dir, store)
}

/// Run on a 2 MiB thread: tokio's `spawn_blocking` default, where the app runs it.
fn run_small_stack(store: &Store, query: String) -> bool {
    let view = store.whole_graph().expect("load");
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn_scoped(scope, move || {
                view.query(&query, QueryDialect::Advanced).is_ok()
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

#[test]
fn a_quoted_deep_nest_under_the_size_cap_does_not_overflow_the_stack() {
    let (dir, store) = graph("quoted");
    let levels = 10_000;
    let query = format!(
        "[:find (pull ?b [*]) :where \"{}x{}\" (task ?b #{{\"TODO\"}})]",
        "(and ".repeat(levels),
        ")".repeat(levels)
    );
    assert!(query.len() < 64 * 1024, "{}", query.len());
    // Refused or answered, never an abort.
    let _ = run_small_stack(&store, query);
    // The guard counts parentheses only; brackets and braces must stay data
    // to every consumer, at any depth under the size cap.
    for (open, close) in [("[", "]"), ("{", "}"), ("#{", "}"), ("(not [", "])")] {
        let levels = 60 * 1024 / (open.len() + close.len()) - 10;
        let query = format!(
            "[:find (pull ?b [*]) :where {}x{} (task ?b #{{\"TODO\"}})]",
            open.repeat(levels),
            close.repeat(levels)
        );
        let _ = run_small_stack(&store, query);
    }
    store.close();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The benign extreme beside the hostile one: a real nest right at the
/// admission bound still answers, and an ordinary query is unchanged.
#[test]
fn deep_but_admitted_and_groups_still_answer() {
    let (dir, store) = graph("benign");
    let levels = 100;
    let query = format!(
        "[:find (pull ?b [*]) :where {}(task ?b #{{\"TODO\"}}){}]",
        "(and ".repeat(levels),
        ")".repeat(levels)
    );
    assert!(run_small_stack(&store, query));
    assert!(run_small_stack(
        &store,
        "[:find (pull ?b [*]) :where (task ?b #{\"TODO\"})]".to_string()
    ));
    store.close();
    let _ = std::fs::remove_dir_all(&dir);
}
