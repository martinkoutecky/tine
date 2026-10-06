//! Public whole-graph read contract.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tine_core::model::{BacklinkFilterTarget, PageEntry, PageKind};
use tine_core::query::ir::{Field, FriendlyPageMatchScope, SortDir, ViewSettings};
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
fn journal_content_days_preserves_prose_while_skipping_unicode_properties() {
    let fixture = Fixture::new();
    let journal = fixture.0.join("journals/2026_09_25.md");
    for (raw, has_content) in [
        // lsdoc (an mldoc transcription) reads `#tag::` as a property key, so this line is a property, not prose.
        ("- #tag:: prose\n", false),
        ("- klíč:: hodnota\n", false),
        ("- key::value\n", true),
    ] {
        std::fs::write(&journal, raw).unwrap();
        let days = fixture.view().journal_content_days();
        assert_eq!(
            days.contains(&tine_store::Day(20260925)),
            has_content,
            "{raw}"
        );
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
            dialect: Default::default(),
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
    let QueryResult::Simple(groups) = view.query("(task TODO)", QueryDialect::Simple).unwrap()
    else {
        panic!("simple result")
    };
    assert!(!groups.is_empty());
    let QueryResult::Advanced(_) = view
        .query(
            "[:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]",
            QueryDialect::Advanced,
        )
        .unwrap()
    else {
        panic!("advanced result")
    };
    let within = PageId::from("pages/archive/Named.md");
    let search = view
        .search(
            &SearchRequest {
                text: "archived".into(),
                within: Some(within.clone()),
                page_limit: 10,
                block_limit: 10,
                explain: false,
                page_match_scope: None,
                page_view: None,
                block_view: None,
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
            page_match_scope: None,
            page_view: None,
            block_view: None,
        },
        &Cancel(Arc::new(AtomicBool::new(true))),
    );
    assert!(matches!(cancelled, Err(QueryError::Cancelled)));
    for bad in ["../x.md", "pages/../../x.md", "/tmp/x.md"] {
        let id = PageId::from(bad);
        assert!(matches!(
            view.search(
                &SearchRequest {
                    text: "x".into(),
                    within: Some(id),
                    page_limit: 1,
                    block_limit: 1,
                    explain: false,
                    page_match_scope: None,
                    page_view: None,
                    block_view: None,
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
fn scoped_search_keeps_block_allowance_when_page_limit_is_large() {
    let fixture = Fixture::new();
    let view = fixture.view();
    let hits = view
        .search(
            &SearchRequest {
                text: "searchable".into(),
                within: Some(PageId::from("pages/Source.md")),
                page_limit: 20_000,
                block_limit: 10,
                explain: false,
                page_match_scope: None,
                page_view: None,
                block_view: None,
            },
            &Cancel(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
    assert!(!hits.hits.is_empty());
}

#[test]
fn public_search_request_routes_page_content_membership() {
    let fixture = Fixture::new();
    let view = fixture.view();
    let cancel = Cancel(Arc::new(AtomicBool::new(false)));
    let search = |page_match_scope| {
        view.search(
            &SearchRequest {
                text: "searchable".into(),
                within: None,
                page_limit: 10,
                block_limit: 0,
                explain: false,
                page_match_scope,
                page_view: None,
                block_view: None,
            },
            &cancel,
        )
        .unwrap()
    };
    let names = search(None);
    let content = search(Some(FriendlyPageMatchScope::Content));
    let both = search(Some(FriendlyPageMatchScope::Both));
    assert!(!names.hits.iter().any(|hit| matches!(hit,
        tine_core::query_plan::QueryHit::Page { page, .. } if page.name == "Source")));
    for result in [content, both] {
        assert!(result.hits.iter().any(|hit| matches!(hit,
            tine_core::query_plan::QueryHit::Page { page, evidence, .. }
            if page.rel_path_str() == "pages/Source.md"
                && evidence.iter().any(|item| item.field == tine_core::query_plan::TextField::VisibleContent))));
    }
}

#[test]
fn public_search_hydrates_authored_properties_only_for_physical_pages() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join("pages/Source.md"),
        "owner:: Mira\n\n- searchable\n",
    )
    .unwrap();
    let view = fixture.view();
    let result = view
        .search(
            &SearchRequest {
                text: "Source".into(),
                within: None,
                page_limit: 10,
                block_limit: 0,
                explain: false,
                page_match_scope: None,
                page_view: None,
                block_view: None,
            },
            &Cancel(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
    assert!(result.hits.iter().any(|hit| matches!(hit,
        tine_core::query_plan::QueryHit::Page { page, row: Some(row), .. }
        if page.rel_path_str() == "pages/Source.md" && row.path == "pages/Source.md"
            && row.properties.iter().any(|(key, value)| key == "owner" && value == "Mira"))));
}

#[test]
fn public_search_sorts_complete_sections_then_samples_them_independently() {
    let fixture = Fixture::new();
    for (name, priority) in [("Alpha A", "C"), ("Alpha B", "B"), ("Alpha Z", "A")] {
        std::fs::write(
            fixture.0.join(format!("pages/{name}.md")),
            format!("- TODO [#{priority}] alpha task\n"),
        )
        .unwrap();
    }
    let view = fixture.view();
    let mut request = SearchRequest {
        text: "alpha".into(),
        within: None,
        page_limit: 2,
        block_limit: 3,
        explain: false,
        page_match_scope: None,
        page_view: Some(ViewSettings {
            sort: vec![(Field("name".into()), SortDir::Desc)],
            sample: Some(1),
            ..Default::default()
        }),
        block_view: Some(ViewSettings {
            sort: vec![(Field("priority".into()), SortDir::Asc)],
            sample: Some(2),
            ..Default::default()
        }),
    };
    let result = view
        .search(&request, &Cancel(Arc::new(AtomicBool::new(false))))
        .unwrap();
    let pages: Vec<_> = result
        .hits
        .iter()
        .filter_map(|hit| match hit {
            tine_core::query_plan::QueryHit::Page { page, .. } => Some(page.name.as_str()),
            _ => None,
        })
        .collect();
    let blocks: Vec<_> = result
        .hits
        .iter()
        .filter_map(|hit| match hit {
            tine_core::query_plan::QueryHit::Block { block, .. } => Some(block.raw.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(pages, ["Alpha Z"]);
    assert_eq!(blocks, ["TODO [#A] alpha task", "TODO [#B] alpha task"]);
    assert!(result.has_more.pages && result.has_more.blocks);

    request.page_view.as_mut().unwrap().sample = Some(0);
    let no_pages = view
        .search(&request, &Cancel(Arc::new(AtomicBool::new(false))))
        .unwrap();
    assert_eq!(
        no_pages
            .hits
            .iter()
            .filter(|hit| matches!(hit, tine_core::query_plan::QueryHit::Page { .. }))
            .count(),
        0
    );
    assert_eq!(
        no_pages
            .hits
            .iter()
            .filter(|hit| matches!(hit, tine_core::query_plan::QueryHit::Block { .. }))
            .count(),
        2
    );
    request.page_view.as_mut().unwrap().sample = Some(1000);
    request.block_view.as_mut().unwrap().sample = Some(1000);
    let capped = view
        .search(&request, &Cancel(Arc::new(AtomicBool::new(false))))
        .unwrap();
    assert_eq!(
        capped
            .hits
            .iter()
            .filter(|hit| matches!(hit, tine_core::query_plan::QueryHit::Page { .. }))
            .count(),
        2
    );
    assert_eq!(
        capped
            .hits
            .iter()
            .filter(|hit| matches!(hit, tine_core::query_plan::QueryHit::Block { .. }))
            .count(),
        3
    );
}

#[test]
fn simple_query_rejects_source_and_nesting_limits() {
    let fixture = Fixture::new();
    let graph = fixture.view();
    let oversized = "x".repeat(tine_core::query::QUERY_SOURCE_MAX_BYTES + 1);
    assert!(matches!(
        graph.query(&oversized, QueryDialect::Simple),
        Err(QueryError::Parse(_))
    ));
    let nested = format!("{}x{}", "(".repeat(129), ")".repeat(129));
    assert!(matches!(
        graph.query(&nested, QueryDialect::Simple),
        Err(QueryError::Parse(_))
    ));
}

#[test]
fn advanced_query_rejects_source_and_nesting_limits() {
    let fixture = Fixture::new();
    let graph = fixture.view();
    let oversized = "x".repeat(tine_core::query::QUERY_SOURCE_MAX_BYTES + 1);
    assert!(matches!(
        graph.query(&oversized, QueryDialect::Advanced),
        Err(QueryError::Parse(_))
    ));
    let nested = format!("{}x{}", "(".repeat(129), ")".repeat(129));
    assert!(matches!(
        graph.query(&nested, QueryDialect::Advanced),
        Err(QueryError::Parse(_))
    ));
}

#[test]
fn journal_fallback_title_resolves_existing_day() {
    let fixture = Fixture::new();
    let graph = fixture.view();
    assert!(matches!(
        graph.resolve("2026-09-25", true),
        Resolved::Existing { id, .. } if id.as_str() == "journals/2026_09_25.md"
    ));
}

// Case twins need a case-sensitive filesystem; Windows folds case, so the
// second write replaces the first file's content under its original name. The
// Windows sibling below asserts that one-file outcome instead.
#[cfg(not(windows))]
#[test]
fn inventory_targets_agree_with_resolve_for_case_twins() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("pages/Twin.md"), "- upper\n").unwrap();
    std::fs::write(fixture.0.join("pages/twin.md"), "- lower\n").unwrap();
    let view = fixture.view();
    let inventory = view.inventory(tine_store::InventoryScope::All);
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

/// Windows sibling of `inventory_targets_agree_with_resolve_for_case_twins`:
/// on a case-insensitive filesystem the lower-case write lands in the existing
/// `Twin.md`, so there is exactly one page, listed once, with no twin, and the
/// inventory target still agrees with `resolve`.
#[cfg(windows)]
#[test]
fn case_twin_writes_fold_into_one_page_on_windows() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("pages/Twin.md"), "- upper\n").unwrap();
    std::fs::write(fixture.0.join("pages/twin.md"), "- lower\n").unwrap();
    let names: Vec<_> = std::fs::read_dir(fixture.0.join("pages"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.eq_ignore_ascii_case("twin.md"))
        .collect();
    assert_eq!(names, ["Twin.md"], "the filesystem folds case");
    assert_eq!(
        std::fs::read_to_string(fixture.0.join("pages/Twin.md")).unwrap(),
        "- lower\n"
    );
    let view = fixture.view();
    let inventory = view.inventory(tine_store::InventoryScope::All);
    let twins: Vec<_> = inventory
        .0
        .iter()
        .filter(|entry| entry.name.eq_ignore_ascii_case("twin"))
        .collect();
    assert_eq!(twins.len(), 1, "one page for the folded name");
    let entry = twins[0];
    let Resolved::Existing { id, others } = &entry.target else {
        panic!("the page should be existing");
    };
    assert_eq!(id.as_str(), "pages/Twin.md");
    assert!(others.is_empty(), "no case twin exists: {others:?}");
    let Resolved::Existing {
        id: resolved,
        others: resolved_others,
    } = view.resolve(&entry.name, false)
    else {
        panic!("resolve should agree with the inventory");
    };
    assert_eq!(&resolved, id);
    assert!(resolved_others.is_empty());
}
