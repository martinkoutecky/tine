//! CM step 2 batch 9 (conformance S10, S12): the query leaves that mean
//! "references A" match A's whole alias group, and every name leaf compares a
//! journal date through the one date answer.
//!
//! - D-20261019-Q8: `[[A]]` / `refs` (and `#A`, TQL `ref`, `direct_ref`)
//!   match the blocks that reference any name of A's alias group; `name` and
//!   `page` compare the literal name (no alias expansion), through the page key.
//! - S12: the page key folds a date spelling onto its journal, so
//!   `(page "2026-10-26")` finds `journals/2026_10_26.md`.
//!
//! Fixtures are the bridge harness's `r7-alias-query` and `r7-page-queries`.
use tine_core::query::ir::{ExecutionContext, QueryRows};
use tine_core::query::registry::Registry;
use tine_core::query::{parse_query_input, QueryInput};
use tine_store::{IrAnswer, IrRequest, Store, WholeGraph};

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, WholeGraph) {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["pages", "journals", "logseq"] {
        std::fs::create_dir_all(dir.path().join(sub)).unwrap();
    }
    for (path, text) in files {
        std::fs::write(dir.path().join(path), text).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    (dir, graph)
}

fn run(graph: &WholeGraph, text: &str, input: QueryInput) -> QueryRows {
    let (query, view) = parse_query_input(
        text,
        input,
        tine_core::date::JournalDate::today(),
        Registry::none(),
    );
    let IrAnswer::Result(result) = graph
        .query_ir(
            IrRequest::Run {
                query: &query,
                view: &view,
                context: &ExecutionContext::default(),
            },
            &tine_store::Cancel::default(),
        )
        .map(|answer| answer.expect("never cancelled"))
        .unwrap()
    else {
        panic!("result")
    };
    result.rows
}

/// Block rows as `page: first line`, sorted.
fn blocks(graph: &WholeGraph, text: &str, input: QueryInput) -> Vec<String> {
    let QueryRows::Block { groups } = run(graph, text, input) else {
        panic!("{text}: block rows")
    };
    let mut rows: Vec<String> = groups
        .iter()
        .flat_map(|group| {
            group.blocks.iter().map(move |block| {
                format!(
                    "{}: {}",
                    group.page,
                    block.raw.lines().next().unwrap_or("")
                )
            })
        })
        .collect();
    rows.sort();
    rows
}

fn pages(graph: &WholeGraph, text: &str) -> Vec<String> {
    let QueryRows::Page { pages } = run(graph, text, QueryInput::MacroTql) else {
        panic!("{text}: page rows")
    };
    let mut names: Vec<String> = pages.into_iter().map(|row| row.name).collect();
    names.sort();
    names
}

const ML: &[(&str, &str)] = &[
    ("pages/Machine Learning.md", "alias:: ML\n\n- intro\n"),
    (
        "pages/notes.md",
        "- reading about [[ML]]\n- also [[Machine Learning]]\n- nothing\n- tagged #ML\n",
    ),
];

fn every_ml_block() -> Vec<String> {
    let mut all = vec![
        "Machine Learning: alias:: ML".to_owned(),
        "Machine Learning: intro".to_owned(),
        "notes: also [[Machine Learning]]".to_owned(),
        "notes: reading about [[ML]]".to_owned(),
        "notes: tagged #ML".to_owned(),
    ];
    all.sort();
    all
}

#[test]
fn a_reference_query_matches_the_whole_alias_group() {
    let _guard = crate::integration_lock();
    let (_dir, graph) = fixture(ML);
    for query in ["[[ML]]", "[[Machine Learning]]", "#ML", "[[ml]]"] {
        assert_eq!(
            blocks(&graph, query, QueryInput::MacroQuery),
            every_ml_block(),
            "{query}"
        );
    }
    for query in ["ref(\"ML\")", "ref(\"Machine Learning\")"] {
        assert_eq!(
            blocks(&graph, query, QueryInput::MacroTql),
            every_ml_block(),
            "{query}"
        );
    }
    // A block's own reference by any name of the group (no path refs).
    let mut direct = vec![
        "Machine Learning: alias:: ML".to_owned(),
        "notes: also [[Machine Learning]]".to_owned(),
        "notes: reading about [[ML]]".to_owned(),
        "notes: tagged #ML".to_owned(),
    ];
    direct.sort();
    assert_eq!(
        blocks(&graph, "direct_ref(\"Machine Learning\")", QueryInput::MacroTql),
        direct
    );
}

#[test]
fn not_takes_the_complement_of_the_whole_group() {
    let _guard = crate::integration_lock();
    let (_dir, graph) = fixture(ML);
    assert_eq!(
        blocks(
            &graph,
            "(and (page \"notes\") (not [[Machine Learning]]))",
            QueryInput::MacroQuery
        ),
        vec!["notes: nothing".to_owned()]
    );
    assert_eq!(
        blocks(&graph, "(not [[ML]])", QueryInput::MacroQuery),
        vec!["notes: nothing".to_owned()]
    );
}

#[test]
fn name_and_page_stay_literal_for_aliases() {
    let _guard = crate::integration_lock();
    let (_dir, graph) = fixture(ML);
    // `page` names a page, not its alias group (D-20261019-Q8).
    assert_eq!(
        blocks(&graph, "(page \"ML\")", QueryInput::MacroQuery),
        Vec::<String>::new()
    );
    // An alias-only name is no page of its own (D-20261019-Q9), and `name`
    // does not expand to the owner.
    assert_eq!(pages(&graph, "@page and name = 'ML'"), Vec::<String>::new());
    assert_eq!(
        pages(&graph, "@page and name = 'Machine Learning'"),
        vec!["Machine Learning".to_owned()]
    );
}

const DAY: &[(&str, &str)] = &[
    ("pages/A.md", "- see [[B]]\n- on [[2026-10-26]]\n- on [[Oct 26th, 2026]]\n"),
    ("journals/2026_10_26.md", "- day\n"),
];

#[test]
fn a_date_spelling_names_its_journal_in_every_leaf() {
    let _guard = crate::integration_lock();
    let (_dir, graph) = fixture(DAY);
    assert_eq!(
        blocks(&graph, "(page \"2026-10-26\")", QueryInput::MacroQuery),
        vec!["Oct 26th, 2026: day".to_owned()]
    );
    assert_eq!(
        pages(&graph, "@page and name = '2026-10-26'"),
        vec!["Oct 26th, 2026".to_owned()]
    );
    let all = vec![
        "A: on [[2026-10-26]]".to_owned(),
        "A: on [[Oct 26th, 2026]]".to_owned(),
        "Oct 26th, 2026: day".to_owned(),
    ];
    for query in ["[[2026-10-26]]", "[[Oct 26th, 2026]]"] {
        assert_eq!(blocks(&graph, query, QueryInput::MacroQuery), all, "{query}");
    }
}
