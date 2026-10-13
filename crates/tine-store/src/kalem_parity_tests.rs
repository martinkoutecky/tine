//! External oracle, independent of Tine's own conformance truth tables.
use std::{collections::BTreeSet, path::Path};

use serde_json::{json, Value};
use tine_core::{
    date::JournalDate,
    query::{
        ir::{ExecutionContext, QueryRows},
        parse_query_input,
        registry::Registry,
        QueryInput,
    },
};

use crate::{IrAnswer, IrRequest, PageId, Store};

// Explicit classifications; the adjacent JSON pins exact observations, so
// both improvement and regression require reviewing this list and the oracle.
// (a) bug, (b) documented divergence, (c) capture quirk, (d) out of scope.
const EXPECTED_DIFFERENCES: &[(&str, &str)] = &[
    ("logseq-md/query/06", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-md/query/14", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-md/query/15", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-md/query/16", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-md/query/19", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-md/query/21", "(b) BACKLOG Advanced Datalog queries: page journal pull is outside the bounded subset; optional page-anchor extension"),
    ("logseq-md/query/23", "(b) OG UI parity: filter-top-level-blocks suppresses matching grandchild when immediate parent matches (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-md/query/24", "(b) OG UI parity: namespace refs match; filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-md/query/25", "(b) BACKLOG Advanced Datalog queries: root-parent page join is outside the bounded subset; optional roots extension"),
    ("logseq-md/backlinks/assets/readme.md", "(c) harness requests Store::page for a non-page asset; OG returns no referring pages; store.rs::validated_page"),
    ("logseq-md/backlinks/pages/contents.md", "(c) Logseq-created page absent on disk, recorded as NotFound rather than skipped"),
    ("logseq-md/backlinks/whiteboards/Design.edn", "(d) whiteboards are not graph text pages; store.rs::validated_page"),
    ("logseq-md/graph_view", "(d) no production graph-view builder; docs/BACKLOG.md Graph view"),
    ("logseq-org/query/07", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/09", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/10", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/12", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/13", "(b) OG UI parity: namespace refs match; filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/18", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/19", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/20", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/22", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/query/24", "(b) BACKLOG Advanced Datalog queries: page journal pull is outside the bounded subset; optional page-anchor extension"),
    ("logseq-org/query/26", "(b) OG UI parity: filter-top-level-blocks suppresses children of matching parents (ADR 0042); Kalem captures unfiltered API hits"),
    ("logseq-org/graph_view", "(d) no production graph-view builder; docs/BACKLOG.md Graph view"),
];

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn set(value: &Value) -> BTreeSet<Vec<String>> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            if let Some(name) = row.as_str() {
                vec![name.to_owned()]
            } else {
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect()
            }
        })
        .collect()
}

fn first_line(raw: &str) -> String {
    let line = raw.lines().next().unwrap_or_default().trim();
    let stars = line.trim_start_matches('*');
    line.strip_prefix("- ")
        .or_else(|| {
            (stars.len() < line.len())
                .then(|| stars.strip_prefix(' '))
                .flatten()
        })
        .unwrap_or(line)
        .to_lowercase()
}

#[test]
fn kalem_logseq_answers() {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tine-core/src/query/fixtures/kalem");
    let today = JournalDate {
        year: 2026,
        month: 10,
        day: 11,
    };
    let mut differences = serde_json::Map::new();
    for (corpus, answers) in [
        ("logseq-md", "answers-0.10.9.json"),
        ("logseq-org", "answers-org-0.10.9.json"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        copy_tree(&fixtures.join(corpus), temp.path());
        let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        let graph = store.whole_graph().unwrap();
        assert!(graph.unreadable_files().is_empty());
        let answers: Value =
            serde_json::from_slice(&std::fs::read(fixtures.join(answers)).unwrap()).unwrap();
        for (n, capture) in answers["queries"].as_array().unwrap().iter().enumerate() {
            let source = capture["query"].as_str().unwrap();
            // WholeGraph::query_ir's production body, including its memo and
            // resolver, with only the sampled civil day supplied by the test.
            let input = if capture["kind"] == "advanced" {
                QueryInput::Advanced
            } else {
                QueryInput::Og
            };
            let (query, view) = parse_query_input(source, input, today, Registry::none());
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
            assert!(!result.exceeded);
            let rows: BTreeSet<Vec<String>> = match &result.rows {
                QueryRows::Block { groups } => groups
                    .iter()
                    .flat_map(|group| {
                        group.blocks.iter().map(|block| {
                            // Org property projections synthesize a drawer; Kalem
                            // identifies a pre-block by the source file's line 0.
                            let raw = if block.page_property {
                                store
                                    .page_named(&group.page, group.kind)
                                    .unwrap()
                                    .unwrap()
                                    .doc
                                    .pre_block
                                    .unwrap()
                            } else {
                                block.raw.clone()
                            };
                            vec![group.page.to_lowercase(), first_line(&raw)]
                        })
                    })
                    .collect(),
                QueryRows::Page { pages } => {
                    pages.iter().map(|p| vec![p.name.to_lowercase()]).collect()
                }
            };
            let want = set(&capture["answer"]);
            if rows != want || !result.report.supported {
                differences.insert(
                    format!("{corpus}/query/{n:02}"),
                    json!({
                        "source": source, "logseq_only": want.difference(&rows).collect::<Vec<_>>(),
                        "tine_only": rows.difference(&want).collect::<Vec<_>>(),
                        "supported": result.report.supported, "report": result.report,
                    }),
                );
            }
        }
        for (path, want) in answers["backlinks"].as_object().unwrap() {
            let page = store.page(&PageId::from(path.as_str()));
            let got = match page {
                Ok(page) => json!(graph
                    .backlinks(&page.doc.name)
                    .unwrap()
                    .iter()
                    .map(|group| group.page.to_lowercase())
                    .collect::<BTreeSet<_>>()),
                Err(error) => json!({"unavailable": format!("{error:?}")}),
            };
            if !got.is_array() || set(&got) != set(want) {
                differences.insert(
                    format!("{corpus}/backlinks/{path}"),
                    json!({"logseq": want, "tine": got}),
                );
            }
        }
        // (d) Graph view is future work (docs/BACKLOG.md, Graph view). There
        // is no production builder to call; preserve the entire request.
        differences.insert(format!("{corpus}/graph_view"), json!({"unavailable": "graph view is not implemented", "logseq": answers["graph_view"]}));
        store.close();
    }
    let got = Value::Object(differences);
    let classified: BTreeSet<_> = EXPECTED_DIFFERENCES
        .iter()
        .map(|(key, reason)| {
            assert!(!reason.is_empty());
            *key
        })
        .collect();
    assert_eq!(
        classified.len(),
        EXPECTED_DIFFERENCES.len(),
        "duplicate classification"
    );
    let expected: Value =
        serde_json::from_str(include_str!("../tests/kalem-expected-differences.json")).unwrap();
    assert_eq!(
        classified,
        expected
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect(),
        "every exception has a classification"
    );
    assert_eq!(
        got,
        expected,
        "classified differences changed:\n{}",
        serde_json::to_string_pretty(&got).unwrap()
    );
}
