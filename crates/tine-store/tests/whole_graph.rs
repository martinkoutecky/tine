//! Public whole-graph read contract while Store adopts the legacy graph.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tine_core::model::{BacklinkFilterTarget, PageKind};
use tine_core::query::QueryExportSpec;
use tine_store::model::Graph;
use tine_store::{Cancel, FacetPolicy, QueryError, Store};

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tine-whole-graph-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("pages")).unwrap();
        std::fs::create_dir_all(path.join("journals")).unwrap();
        std::fs::write(path.join("pages/Target.md"), "- target\n").unwrap();
        std::fs::write(path.join("journals/2026_09_25.md"), "- journal note\n").unwrap();
        std::fs::write(
            path.join("pages/Source.md"),
            "icon:: ⭐\n\n- [[Target]] alpha searchable\n  id:: aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n  color:: blue\n- Target plain mention\n- ((aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa))\n- template example\n  template:: Example\n",
        )
        .unwrap();
        Self(path)
    }

    fn view(&self) -> tine_store::WholeGraph {
        Store::from_legacy(Arc::new(Graph::open(&self.0)))
            .whole_graph()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn all_whole_graph_questions_use_the_public_view() {
    let fixture = Fixture::new();
    let view = fixture.view();
    let rev = view.rev();
    assert!(serde_json::to_string(&rev).unwrap().starts_with('"'));
    assert_eq!(view.clone().rev(), rev);

    let backlinks = view.backlinks("Target").unwrap();
    assert!(backlinks.iter().any(|group| group.page == "Source"));
    let unlinked = view.unlinked_references("Target").unwrap();
    assert!(unlinked.iter().any(|group| group.page == "Source"));
    let source_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string();
    let blocks = view.blocks(&[source_id.clone(), "missing".into()]).unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].as_ref().unwrap().page, "Source");
    assert!(blocks[1].is_none());
    let context = view
        .backlink_filter_context(
            "Target",
            &[BacklinkFilterTarget {
                page: "Source".into(),
                kind: PageKind::Page,
                block_id: blocks[0].as_ref().unwrap().blocks[0].id.clone(),
            }],
        )
        .unwrap();
    assert!(!context.truncated);
    assert!(!context.entries.is_empty());
    assert!(view.preview_block(&source_id, 20).unwrap().is_some());
    assert!(!view.block_referrers(&source_id).unwrap().is_empty());
    assert!(
        view.block_ref_counts()
            .get(&source_id)
            .copied()
            .unwrap_or(0)
            > 0
    );
    assert!(view
        .complete_page_names("Source", 10)
        .iter()
        .any(|p| p.name == "Source"));
    assert!(!view
        .find_blocks("searchable", 10, &Cancel(Arc::new(AtomicBool::new(false))))
        .unwrap()
        .is_empty());
    let export = view
        .export_query_subtrees(&[QueryExportSpec {
            key: "one".into(),
            query: "(page Target)".into(),
            advanced: false,
        }])
        .unwrap();
    assert_eq!(export.results.len(), 1);
    assert!(export.results[0].total > 0);
    assert!(!view
        .property_facets(FacetPolicy::Budgeted)
        .unwrap()
        .is_empty());
    assert!(!view
        .property_facets(FacetPolicy::Truncated)
        .unwrap()
        .is_empty());
    assert!(view.templates().iter().any(|t| t.name == "Example"));
    let icons = view.page_icons(&["Source".into()]);
    assert_eq!(icons.get("Source").map(String::as_str), Some("⭐"));
    assert!(view.journal_content_days().contains(&20260925));
}

#[test]
fn bounded_and_cancelled_answers_are_typed() {
    let fixture = Fixture::new();
    let view = fixture.view();
    assert!(matches!(
        view.blocks(&vec!["missing".into(); 20_001]),
        Err(QueryError::ResultTooLarge { .. })
    ));
    assert!(matches!(
        view.find_blocks("searchable", 10, &Cancel(Arc::new(AtomicBool::new(true)))),
        Err(QueryError::Cancelled)
    ));

    std::fs::write(
        fixture.0.join("pages/Large.md"),
        "- [[Target]] many\n".repeat(20_001),
    )
    .unwrap();
    let fresh = fixture.view();
    assert!(matches!(
        fresh.backlinks("Target"),
        Err(QueryError::ResultTooLarge { .. })
    ));
}
