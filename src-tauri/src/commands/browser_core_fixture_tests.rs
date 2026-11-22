//! Compare the shared read adapters on a disk Store with the desktop captures.

use super::*;
use serde_json::Value;
use tine_core::query::ir::ExecutionContext;
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::inventory_wire::page_backlink_counts_wire;
use tine_graph_features::page_read_wire;
use tine_graph_features::query_wire;
use tine_graph_features::search::{self, SearchLanes};
use tine_store::Cancel;

const UUID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

#[test]
fn browser_core_command_wires() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/browser-core-reads.json"
    ))
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let files: Vec<(String, String)> = serde_json::from_value(fixture["files"].clone()).unwrap();
    for (path, text) in files {
        let file = root.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        std::fs::File::open(file)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
            .unwrap();
    }
    let (store, _, _) = tine_store::Store::open(root.path(), Default::default()).unwrap();
    let view = store.whole_graph_reconciled().unwrap();
    let mut checked = 0;
    let mut record = |name: &str, wire: String| {
        assert_eq!(
            Some(wire.as_str()),
            fixture["expected"][name].as_str(),
            "{name}"
        );
        checked += 1;
    };
    macro_rules! record {
        ($name:literal, $answer:expr) => {
            record($name, serde_json::to_string(&$answer).unwrap())
        };
    }

    record!(
        "graph_meta",
        graph_read_wire::graph_meta_wire("/tine-memory-graph".into(), &store.config(),)
    );
    let mut modified = page_modified_wire(&view);
    assert_eq!(modified.rev, String::from(view.rev()));
    modified.rev = "0".into();
    record!("page_modified", modified);
    let (files, references, rev) = page_backlink_counts_wire(&view);
    assert_eq!(rev, String::from(view.rev()));
    record!("page_backlink_counts", (files, references, "0"));
    record!("resolve_page", resolve_name(&view, "Org Notes", false));
    record!(
        "resolve_namespace",
        resolve_name(&view, "Team/Child", false)
    );
    record!("page_alias_group", alias_group_wire(&view, "Shortcut"));
    let source = || PageSource::Live(&store);
    record!(
        "get_page",
        page_read_wire::get_page(source(), "Shortcut", PageKind::Page).unwrap()
    );
    record!(
        "get_page_by_path",
        page_read_wire::get_page_by_path(source(), PageId::from("pages/Notes.org".to_owned()))
            .unwrap()
    );
    for (name, before) in [
        ("journal_feed_page", None),
        ("journal_feed_page_before", Some(20260627)),
    ] {
        let mut feed = page_read_wire::journal_feed_page(source(), 1, before, None, true).unwrap();
        assert_eq!(
            feed.as_of_day,
            tine_core::date::JournalDate::today().ordinal_key()
        );
        let wire = fixture["expected"][name].as_str().unwrap();
        feed.as_of_day = serde_json::from_str::<Value>(wire).unwrap()["as_of_day"]
            .as_i64()
            .unwrap();
        record(name, serde_json::to_string(&feed).unwrap());
    }
    record!(
        "journal_content_days",
        graph_read_wire::journal_content_days(&view)
    );
    record!(
        "page_icons",
        graph_read_wire::page_icons(&view, &["Shortcut".into(), "Missing".into()])
    );
    let backlinks = graph_read_wire::backlinks(&view, "Target")
        .map_err(reference_error)
        .unwrap();
    let targets = backlinks
        .iter()
        .flat_map(|group| {
            group.blocks.iter().map(|block| BacklinkFilterTarget {
                page: group.page.clone(),
                kind: group.kind,
                block_id: block.id.clone(),
            })
        })
        .collect::<Vec<_>>();
    record!("get_backlinks", backlinks);
    record!(
        "get_backlink_filter_context",
        graph_read_wire::backlink_filter_context(&view, "Target", &targets).unwrap()
    );
    record!(
        "get_unlinked_refs",
        graph_read_wire::unlinked_refs(&view, "Target")
            .map_err(reference_error)
            .unwrap()
    );
    record!(
        "get_block_ref_counts",
        graph_read_wire::block_ref_counts(&view)
    );
    record!(
        "get_block_referrers",
        graph_read_wire::block_referrers(&view, UUID).unwrap()
    );
    record!(
        "resolve_blocks",
        graph_read_wire::resolve_blocks(&view, &[UUID.into(), "missing".into()]).unwrap()
    );
    record!(
        "preview_block",
        graph_read_wire::preview_block(&view, UUID, 1).unwrap()
    );
    let lanes = SearchLanes::default();
    record!(
        "search",
        search::find_blocks_sequenced(&view, &lanes, "alpha searchable", 5, None, None).unwrap()
    );
    record!(
        "capture_quick_switch",
        capture_completion(&view, "Short", 20, false)
    );
    for (name, scope) in [
        ("query_facet_keys", r#"{"keys":true}"#),
        ("query_facet_values", r#"{"key":"color"}"#),
    ] {
        let scope: AutocompleteFacetScope = serde_json::from_str(scope).unwrap();
        record(
            name,
            serde_json::to_string(&graph_read_wire::query_facets(
                &view,
                scope.into_policy().unwrap(),
            ))
            .unwrap(),
        );
    }
    let parsed = query_wire::parse(&view, "(task TODO)", QueryTextDialect::Og, &[]).unwrap();
    let run = query_wire::run_wire(
        &view,
        &parsed.query,
        &parsed.view,
        &ExecutionContext::default(),
        &Cancel::default(),
    )
    .unwrap();
    record!("parse_query", parsed);
    record!("query_run", run);
    record!(
        "run_graph_search",
        search::run_graph_search_sequenced(
            &view,
            &lanes,
            "alpha".into(),
            5,
            5,
            None,
            None,
            true,
            None,
            None,
            None,
            None,
            false
        )
        .unwrap()
    );

    assert_eq!(checked, fixture["expected"].as_object().unwrap().len());
}
