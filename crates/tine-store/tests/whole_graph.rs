//! Public whole-graph read contract.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tine_core::model::{BacklinkFilterTarget, PageEntry, PageKind};
use tine_core::query::QueryExportSpec;
use tine_store::{
    Area, Cancel, FacetPolicy, PageId, QueryDialect, QueryError, QueryResult, Resolved,
    SearchRequest, Store, StoreError,
};

struct Fixture(std::path::PathBuf);

#[test]
fn page_entry_empty_path_keeps_legacy_wire_form() {
    let entry = PageEntry {
        name: "Virtual".into(),
        kind: PageKind::Page,
        date_key: None,
        rel_path: None,
        path: std::path::PathBuf::new(),
    };
    let value = serde_json::to_value(&entry).unwrap();
    assert_eq!(value["path"], "");
    let decoded: PageEntry = serde_json::from_value(value).unwrap();
    assert!(decoded.rel_path.is_none());
    let real: PageEntry = serde_json::from_str(
        r#"{"name":"Real","kind":"page","date_key":null,"path":"pages/Real.md"}"#,
    )
    .unwrap();
    assert_eq!(real.rel_path.unwrap().as_str(), "pages/Real.md");
}

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
        Store::open(&self.0, Default::default())
            .unwrap()
            .0
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
    assert!(view
        .journal_content_days()
        .contains(&tine_store::Day(20260925)));
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

#[test]
fn identity_resolution_and_wire_paths() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("pages/a.org"), "- org twin\n").unwrap();
    std::fs::write(fixture.0.join("pages/a.md"), "- md twin\n").unwrap();
    std::fs::write(
        fixture.0.join("pages/AliasOwner.md"),
        "alias:: Shortcut\n\n- owner\n",
    )
    .unwrap();
    std::fs::write(
        fixture.0.join("journals/2026_09_25.org"),
        "- journal twin\n",
    )
    .unwrap();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    let Resolved::Existing { id, others } = view.resolve("a", false) else {
        panic!("a must exist")
    };
    assert_eq!(serde_json::to_string(&id).unwrap(), "\"pages/a.md\"");
    assert_eq!(
        others[0].file(),
        store.file_id(Area::Pages, "a.org").unwrap()
    );
    let file = id.file();
    assert_eq!(serde_json::to_string(&file).unwrap(), "\"pages/a.md\"");
    assert_eq!(
        serde_json::from_str::<tine_store::FileId>("\"pages/a.md\"").unwrap(),
        file
    );
    let decoded: PageId = serde_json::from_str("\"pages/a.md\"").unwrap();
    assert_eq!(decoded, id);
    let Resolved::Existing { id, others } = view.resolve("Sep 25th, 2026", true) else {
        panic!("journal must exist")
    };
    assert_eq!(id.as_str(), "journals/2026_09_25.md");
    assert_eq!(others[0].as_str(), "journals/2026_09_25.org");
    let Resolved::Alias { owners } = view.resolve("Shortcut", false) else {
        panic!("alias must resolve")
    };
    assert_eq!(owners[0].as_str(), "pages/AliasOwner.md");
    let Resolved::Absent { id } = view.resolve("Missing", false) else {
        panic!("missing must be absent")
    };
    assert_eq!(id.as_str(), "pages/Missing.md");
}

#[test]
fn query_and_scoped_search_use_page_identity() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join("pages/Named.md"),
        "title:: Display Title\nalias:: Shortcut\n\n- TODO exact owner\n",
    )
    .unwrap();
    std::fs::create_dir_all(fixture.0.join("pages/archive")).unwrap();
    std::fs::write(
        fixture.0.join("pages/archive/Named.md"),
        "- TODO archived duplicate\n",
    )
    .unwrap();
    let store = Store::open(&fixture.0, Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    let QueryResult::Simple(groups) = view
        .query("(task TODO)", QueryDialect::Simple, None)
        .unwrap()
    else {
        panic!("simple result")
    };
    assert!(!groups.is_empty());
    for name in ["Named", "Shortcut"] {
        let id = match view.resolve(name, false) {
            Resolved::Existing { id, .. } | Resolved::Absent { id } => id,
            Resolved::Alias { owners } => owners[0].clone(),
        };
        let QueryResult::Advanced(actual) = view
            .query(
                "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]",
                QueryDialect::Advanced,
                Some(&id),
            )
            .unwrap()
        else {
            panic!("advanced result")
        };
        let QueryResult::Advanced(expected) = view
            .query(
                "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]",
                QueryDialect::Advanced,
                None,
            )
            .unwrap()
        else {
            panic!("advanced result")
        };
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    let within = PageId::from("pages/archive/Named.md");
    let search = view
        .search(
            &SearchRequest {
                text: "archived".into(),
                within: Some(within.clone()),
                page_limit: 10,
                block_limit: 10,
                explain: false,
            },
            &Cancel(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
    assert!(search.hits.iter().all(
        |hit| matches!(hit, tine_core::query_plan::QueryHit::Block { path, .. } if path == &within)
    ));
    assert!(!search.hits.is_empty());
    let cancelled = view.search(
        &SearchRequest {
            text: "archived".into(),
            within: Some(within),
            page_limit: 10,
            block_limit: 10,
            explain: false,
        },
        &Cancel(Arc::new(AtomicBool::new(true))),
    );
    assert!(matches!(cancelled, Err(QueryError::Cancelled)));
    for bad in ["../x.md", "pages/../../x.md", "/tmp/x.md"] {
        let id = PageId::from(bad);
        assert!(matches!(
            view.query("(task TODO)", QueryDialect::Simple, Some(&id)),
            Err(QueryError::InvalidTarget(_))
        ));
        assert!(matches!(
            view.search(
                &SearchRequest {
                    text: "x".into(),
                    within: Some(id),
                    page_limit: 1,
                    block_limit: 1,
                    explain: false
                },
                &Cancel(Arc::new(AtomicBool::new(false)))
            ),
            Err(QueryError::InvalidTarget(_))
        ));
        assert!(matches!(
            store.file_id(Area::Pages, bad),
            Err(StoreError::InvalidTarget(_))
        ));
    }
}

#[test]
fn simple_query_rejects_source_and_nesting_limits() {
    let fixture = Fixture::new();
    let graph = fixture.view();
    let oversized = "x".repeat(tine_core::query::QUERY_SOURCE_MAX_BYTES + 1);
    assert!(matches!(
        graph.query(&oversized, QueryDialect::Simple, None),
        Err(QueryError::Parse(_))
    ));
    let nested = format!("{}x{}", "(".repeat(65), ")".repeat(65));
    assert!(matches!(
        graph.query(&nested, QueryDialect::Simple, None),
        Err(QueryError::Parse(_))
    ));
}

#[test]
fn inventory_targets_agree_with_resolve_for_case_twins() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("pages/Twin.md"), "- upper\n").unwrap();
    std::fs::write(fixture.0.join("pages/twin.md"), "- lower\n").unwrap();
    let view = fixture.view();
    let inventory = view.inventory();
    let twins: Vec<_> = inventory
        .0
        .iter()
        .filter(|entry| entry.name.eq_ignore_ascii_case("twin"))
        .collect();
    assert!(!twins.is_empty());
    for entry in twins {
        let existing = |target: &Resolved| match target {
            Resolved::Existing { id, others } => Some((
                id.as_str().to_string(),
                others
                    .iter()
                    .map(|id| id.as_str().to_string())
                    .collect::<Vec<_>>(),
            )),
            _ => None,
        };
        let resolved = view.resolve(&entry.name, false);
        assert_eq!(
            existing(&entry.target),
            existing(&resolved),
            "inventory entry {:?}",
            entry.name
        );
        let Resolved::Existing { others, .. } = &entry.target else {
            panic!("twin should be an existing page");
        };
        assert_eq!(others.len(), 1, "the other case twin is listed");
    }
}
