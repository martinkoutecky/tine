//! Cost probes drive the production Store → query_ir entry, never copy content.
use super::WORK;
use crate::{IrAnswer, IrRequest, OpenOptions, Store};
use tine_core::date::JournalDate;
use tine_core::query::ir::ExecutionContext;
use tine_core::query::{parse_query_text, QueryDialect};

fn probe(root: &std::path::Path, expected_nodes: Option<usize>) {
    let (store, _, _) = Store::open(root, OpenOptions::default()).unwrap();
    let graph = store.whole_graph().unwrap();
    for relation in ["parent", "ancestors", "descendants"] {
        let source = format!("any({relation}, content match 'QE5CostProbeNoMatch')");
        let (query, view) = parse_query_text(&source, QueryDialect::Tql, JournalDate::today());
        assert!(!query.is_invalid(), "{:?}", query.diagnostics);
        WORK.with(|work| work.set((0, 0, 0)));
        let start = std::time::Instant::now();
        let answer = graph
            .query_ir(IrRequest::Run {
                query: &query,
                view: &view,
                context: &ExecutionContext::none(),
            })
            .unwrap();
        let IrAnswer::Result(answer) = answer else {
            panic!("result")
        };
        assert_eq!(answer.total, 0);
        let (nodes, predicates, edges) = WORK.with(|work| work.get());
        assert_eq!(
            predicates, nodes,
            "I-25: imitate hierarchy.rs's one predicate per row fold"
        );
        assert!(edges <= nodes);
        if let Some(expected) = expected_nodes {
            assert_eq!(nodes, expected);
        }
        eprintln!(
            "QE5 {relation}: nodes={nodes}, predicates={predicates}, edges={edges}, elapsed_us={}",
            start.elapsed().as_micros()
        );
    }
}

#[test]
fn hierarchy_work_is_linear_for_small_wide_and_deep_pages() {
    for (nodes, deep) in [(1, false), (60, false), (120, true)] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        let text: String = (0..nodes)
            .map(|at| {
                format!(
                    "{}- node{at}\n",
                    "\t".repeat(if deep { at } else { usize::from(at > 0) })
                )
            })
            .collect();
        std::fs::write(dir.path().join("pages/Probe.md"), text).unwrap();
        probe(dir.path(), Some(nodes));
    }
}

#[test]
#[ignore = "explicit anonymized-graph cost acceptance, outside the fast loop"]
fn hierarchy_anonymized_graph_work() {
    probe(
        std::path::Path::new("/home/koutecky/research/logseq-anonymized"),
        None,
    );
}
