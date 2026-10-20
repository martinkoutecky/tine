//! Card BhkrU (#710 follow-up, GA5): the TQL condition "a block mentioning a
//! day after today", `any(direct_refs, day > today)`, over a fixture graph,
//! across a day boundary, and against what the opt-in `{{upcoming}}` section
//! counts (`InventoryScope::ReferencedJournals`, the feed's upcoming days).

use std::collections::BTreeSet;
use std::path::Path;

use tine_core::date::JournalDate;
use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::{parse_query_input, registry::Registry, QueryInput};

use crate::{IrAnswer, IrRequest, Store};

const TODAY: JournalDate = JournalDate {
    year: 2026,
    month: 10,
    day: 19,
};

const PLANS: &str = "\
- TODO dentist [[Oct 22nd, 2026]]
- party #[[Oct 24th, 2026]]
- review [[Oct 18th, 2026]]
- standup [[Oct 19th, 2026]]
- iso spelling [[2026-10-26]]
- parent [[Oct 22nd, 2026]]
\t- child only inherits its parent's reference
- TODO call the garage
  SCHEDULED: <2026-10-21 Wed>
- tomorrow [[Oct 20th, 2026]]
- a namespaced day [[Oct 28th, 2026/agenda]]
";

fn fixture(root: &Path) {
    for dir in ["journals", "pages", "logseq"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("logseq/config.edn"), "{}\n").unwrap();
    std::fs::write(root.join("pages/Plans.md"), PLANS).unwrap();
    std::fs::write(root.join("pages/Notes.md"), "- nothing dated [[Project]]\n").unwrap();
    // A future journal FILE that nothing references: neither the query nor
    // Upcoming lists it (its own blocks mention no day).
    std::fs::write(
        root.join("journals/2026_10_23.md"),
        "- written ahead, mentions nothing\n",
    )
    .unwrap();
}

/// First lines of the block rows `tql` matches on `today`, sorted.
fn rows(graph: &crate::WholeGraph, tql: &str, today: JournalDate) -> Vec<String> {
    let (query, view) = parse_query_input(tql, QueryInput::Tql, today, Registry::none());
    assert!(!query.is_invalid(), "{tql}: {:?}", query.diagnostics);
    let IrAnswer::Result(result) = crate::query::exec::query_ir_on_day(
        &graph.graph,
        IrRequest::Run {
            query: &query,
            view: &view,
            context: &ExecutionContext::none(),
        },
        today,
    )
    .unwrap() else {
        panic!("run must return query rows")
    };
    assert!(result.report.supported, "{:?}", result.report);
    let QueryRows::Block { groups } = &result.rows else {
        panic!("block rows expected")
    };
    let mut out: Vec<String> = groups
        .iter()
        .flat_map(|group| group.blocks.iter())
        .map(|block| block.raw.lines().next().unwrap_or_default().to_owned())
        .collect();
    out.sort();
    out
}

fn upcoming_days(graph: &crate::WholeGraph, today: JournalDate) -> BTreeSet<i64> {
    graph
        .inventory(crate::InventoryScope::ReferencedJournals)
        .0
        .iter()
        .filter_map(|entry| entry.day)
        .map(|day| day.0)
        .filter(|day| *day > today.ordinal_key())
        .collect()
}

#[test]
fn bhkru_blocks_mentioning_a_day_after_today_on_a_fixture_graph() {
    let temp = tempfile::tempdir().unwrap();
    fixture(temp.path());
    let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
    let graph = store.whole_graph().unwrap();
    let want = vec![
        "TODO dentist [[Oct 22nd, 2026]]",
        "a namespaced day [[Oct 28th, 2026/agenda]]",
        "iso spelling [[2026-10-26]]",
        "parent [[Oct 22nd, 2026]]",
        "party #[[Oct 24th, 2026]]",
        "tomorrow [[Oct 20th, 2026]]",
    ];
    assert_eq!(rows(&graph, "any(direct_refs, day > today)", TODAY), want);
    // The same condition combined with an ordinary one.
    assert_eq!(
        rows(
            &graph,
            "task = 'TODO' and any(direct_refs, day > today)",
            TODAY
        ),
        vec!["TODO dentist [[Oct 22nd, 2026]]"]
    );
    // What `{{upcoming}}` lists for this graph: the same days. A SCHEDULED
    // date is not a reference in either (the Scheduled & Deadline section owns
    // it), and a future journal file nobody mentions is in neither.
    assert_eq!(
        upcoming_days(&graph, TODAY),
        BTreeSet::from([20261020, 20261022, 20261024, 20261026, 20261028])
    );
    store.close();
}

#[test]
fn bhkru_today_is_the_evaluation_day_not_a_day_pinned_in_the_query() {
    let temp = tempfile::tempdir().unwrap();
    fixture(temp.path());
    let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
    let graph = store.whole_graph().unwrap();
    let query = "any(direct_refs, day > today)";
    let before = rows(&graph, query, TODAY);
    assert!(before.contains(&"tomorrow [[Oct 20th, 2026]]".to_owned()));
    // The next local day, through the same snapshot and its answer memo: the
    // day that was "tomorrow" is today now and no longer after it.
    let after = rows(&graph, query, TODAY.add_days(1));
    assert!(!after.contains(&"tomorrow [[Oct 20th, 2026]]".to_owned()));
    assert_eq!(after.len(), before.len() - 1, "{after:?}");
    // And back: the first day's answer was not overwritten by the second.
    assert_eq!(rows(&graph, query, TODAY), before);
    // A week on, only the Oct 28th mention is still ahead.
    assert_eq!(
        rows(&graph, query, TODAY.add_days(7)),
        vec!["a namespaced day [[Oct 28th, 2026/agenda]]"]
    );
    store.close();
}

/// Where the condition and `{{upcoming}}` answer differently, pinned so a
/// change to either is a reviewed decision (notes: og-tql-upcoming.md).
/// 1. A date-named ORDINARY page (`pages/Oct 30th, 2026.md`) keeps ordinary
///    identity in Upcoming (`referenced_journal_days`); the condition reads
///    the reference name alone, as Logseq's `:block/journal-day` join does.
/// 2. A spelling only in the configured FILENAME format counts in Upcoming
///    (the graph's journal format); the condition reads titles first and the
///    safe defaults (gh690: a custom filename format must not turn an
///    ordinary reference into a journal reference).
#[test]
fn bhkru_where_the_condition_and_upcoming_differ() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for dir in ["journals", "pages", "logseq"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(
        root.join("logseq/config.edn"),
        "{:journal/file-name-format \"dd.MM.yyyy\"}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/Plans.md"),
        "- ordinary page named like a day [[Oct 30th, 2026]]\n- filename spelling [[01.11.2026]]\n- title spelling [[Nov 2nd, 2026]]\n",
    )
    .unwrap();
    std::fs::write(root.join("pages/Oct 30th, 2026.md"), "- an ordinary page\n").unwrap();
    let (store, _, _) = Store::open(root, Default::default()).unwrap();
    let graph = store.whole_graph().unwrap();
    assert_eq!(
        rows(&graph, "any(direct_refs, day > today)", TODAY),
        vec![
            "ordinary page named like a day [[Oct 30th, 2026]]",
            "title spelling [[Nov 2nd, 2026]]",
        ]
    );
    assert_eq!(
        upcoming_days(&graph, TODAY),
        BTreeSet::from([20261101, 20261102])
    );
    store.close();
}

/// Card hB3Lv8: `query_run` stamps a day-relative answer with the day it was
/// resolved for, from the memo as from a fresh run, so a window left open
/// overnight knows to re-ask it; a date-free answer carries no stamp and is
/// never re-asked for a new day.
#[test]
fn hb3lv8_a_day_relative_answer_names_its_day_and_a_date_free_one_does_not() {
    let temp = tempfile::tempdir().unwrap();
    fixture(temp.path());
    let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
    let graph = store.whole_graph().unwrap();
    let as_of = |tql: &str, today: JournalDate| {
        let (query, view) = parse_query_input(tql, QueryInput::Tql, today, Registry::none());
        let IrAnswer::Result(result) = crate::query::exec::query_ir_on_day(
            &graph.graph,
            IrRequest::Run {
                query: &query,
                view: &view,
                context: &ExecutionContext::none(),
            },
            today,
        )
        .unwrap() else {
            panic!("run must return query rows")
        };
        result.as_of_day
    };
    let upcoming = "any(direct_refs, day > today)";
    assert_eq!(as_of(upcoming, TODAY), Some(20261019));
    // Served from the memo the second time: still stamped.
    assert_eq!(as_of(upcoming, TODAY), Some(20261019));
    let tomorrow = TODAY.add_days(1);
    assert_eq!(as_of(upcoming, tomorrow), Some(20261020));
    assert_eq!(as_of("task = 'TODO'", TODAY), None);
    assert_eq!(
        as_of("scheduled between '2026-10-01' and '2026-10-31'", TODAY),
        None
    );
}
