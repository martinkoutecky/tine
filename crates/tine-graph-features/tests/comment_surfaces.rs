//! H9TvA / D-20261113-CO: marked comments and whole-thread omission.
use std::fs;
use tine_core::query::ir::FriendlyPageMatchScope;
use tine_core::query::wire_parse::QueryTextDialect;
use tine_core::query_plan::QueryHit;
use tine_graph_features::{print, publish_query, quick_share};
use tine_store::{Cancel, SearchRequest, Store};

#[test]
fn detached_export_hydration_carries_comment_roots_and_replies() {
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::create_dir_all(scratch.path().join("pages")).unwrap();
    fs::write(scratch.path().join("pages/Source.md"), "- Body\n  tine.quote:: root is body\n  - TODO COMMENTMARKER\n    id:: comment-id\n    tine.quote:: Body\n    - DONE REPLYMARKER\n      id:: reply-id\n- Ordinary\n").unwrap();
    let store = Store::open(scratch.path(), Default::default()).unwrap().0;
    let whole = store.whole_graph().unwrap();
    let mut membership = Vec::new();
    for (id, query) in [("comment-id", "(task TODO)"), ("reply-id", "(task DONE)")] {
        let preview = whole.preview_block(id, 100).unwrap().unwrap();
        let batch = whole
            .export_query_subtrees(&[tine_core::query::QueryExportSpec {
                key: id.into(),
                query: query.into(),
                dialect: Default::default(),
            }])
            .unwrap();
        assert_eq!(
            (
                batch.results[0].shown,
                batch.results[0].total,
                batch.results[0].omitted_nodes
            ),
            (1, 1, 0)
        );
        for block in [
            &preview.group.blocks[0],
            &batch.results[0].groups[0].blocks[0],
        ] {
            membership.push((
                id,
                serde_json::to_value(block).unwrap()["comment"]
                    .as_bool()
                    .unwrap_or(false),
            ));
        }
    }
    let body = whole.corpus(Some("Source"), true).pages[0].document.roots[0]
        .uuid
        .to_string();
    let preview = whole.preview_block(&body, 100).unwrap().unwrap();
    assert_ne!(
        serde_json::to_value(&preview.group.blocks[0]).unwrap()["comment"],
        true
    );
    assert_eq!(
        membership,
        [
            ("comment-id", true),
            ("comment-id", true),
            ("reply-id", true),
            ("reply-id", true)
        ]
    );
}

/// Iif8k: reference-panel rows are shallow, detached roots. Their DTO must say
/// whether the source block belongs to a comment thread so Copy/Export labels
/// or omits it; a root carrying `tine.quote` is body text.
#[test]
fn reference_rows_carry_comment_roots_and_replies() {
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::create_dir_all(scratch.path().join("pages")).unwrap();
    fs::write(scratch.path().join("pages/Source.md"), "- ROOTQUOTE [[Target]] Target ((6a1b2c3d-0000-4000-8000-0000000000aa))\n  tine.quote:: root is body\n  - Body\n    - COMMENTMARKER [[Target]] Target ((6a1b2c3d-0000-4000-8000-0000000000aa))\n      tine.quote:: Body\n      - REPLYMARKER [[Target]] Target ((6a1b2c3d-0000-4000-8000-0000000000aa))\n  - ORDINARY [[Target]] Target ((6a1b2c3d-0000-4000-8000-0000000000aa))\n").unwrap();
    fs::write(
        scratch.path().join("pages/Target.md"),
        "- Referenced\n  id:: 6a1b2c3d-0000-4000-8000-0000000000aa\n",
    )
    .unwrap();
    let store = Store::open(scratch.path(), Default::default()).unwrap().0;
    let whole = store.whole_graph().unwrap();
    let membership = |groups: &[tine_core::model::RefGroup]| {
        let mut rows: Vec<(String, bool)> = groups
            .iter()
            .flat_map(|group| &group.blocks)
            .map(|block| {
                let json = serde_json::to_value(block).unwrap();
                (
                    block.raw.split_whitespace().next().unwrap().to_owned(),
                    json["comment"].as_bool().unwrap_or(false),
                )
            })
            .collect();
        rows.sort();
        rows
    };
    let expected = [
        ("COMMENTMARKER".to_owned(), true),
        ("ORDINARY".to_owned(), false),
        ("REPLYMARKER".to_owned(), true),
        ("ROOTQUOTE".to_owned(), false),
    ];
    assert_eq!(membership(&whole.backlinks("Target").unwrap()), expected);
    assert_eq!(
        membership(&whole.unlinked_references("Target").unwrap()),
        expected
    );
    assert_eq!(
        membership(
            &whole
                .block_referrers("6a1b2c3d-0000-4000-8000-0000000000aa")
                .unwrap()
        ),
        expected
    );
}

#[test]
fn detached_macros_keep_native_print_annotation_context() {
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::create_dir_all(scratch.path().join("pages")).unwrap();
    fs::write(scratch.path().join("pages/Source.md"), "public:: true\n- Body\n  - TODO COMMENTMARKER\n    id:: 6a1b2c3d-0000-4000-8000-000000000009\n    tine.quote:: Body\n    - DONE REPLYMARKER\n      id:: 6a1b2c3d-0000-4000-8000-000000000010\n").unwrap();
    fs::write(scratch.path().join("pages/Output.md"), "public:: true\n- Output body\n- {{embed ((6a1b2c3d-0000-4000-8000-000000000009))}}\n- {{embed ((6a1b2c3d-0000-4000-8000-000000000010))}}\n- {{query (task TODO)}}\n- {{query (task DONE)}}\n").unwrap();
    let store = Store::open(scratch.path(), Default::default()).unwrap().0;
    for include_comments in [true, false] {
        let html = print::page_print_html(
            &store,
            "Output",
            print::PrintOpts {
                include_comments,
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(html.contains("Output body"));
        for marker in ["COMMENTMARKER", "REPLYMARKER"] {
            assert_eq!(html.contains(marker), include_comments, "{marker}");
        }
        if include_comments {
            assert_eq!(html.matches("<aside class=\"comment\">").count(), 6);
        }
    }
}

#[test]
fn comments_off_keeps_page_property_query_answer() {
    let _integration_guard = crate::integration_lock();
    comments_off_keeps_header_answer(
        "Header export",
        "(property status exported-state)",
        "exported-state",
        "page_property",
    );
}

#[test]
fn comments_off_keeps_preamble_query_answer() {
    let _integration_guard = crate::integration_lock();
    comments_off_keeps_header_answer(
        "Preamble export",
        "[[PreambleTarget]]",
        "Prose",
        "page_preamble",
    );
}

fn comments_off_keeps_header_answer(name: &str, argument: &str, marker: &str, flag: &str) {
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = scratch.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(
        root.join("pages/Header.md"),
        "status:: exported-state\n\n- Ordinary\n",
    )
    .unwrap();
    fs::write(
        root.join("pages/Preamble.md"),
        "Prose [[PreambleTarget]]\n\n- Ordinary\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let request = publish_query::QueryExportRequest {
        argument: argument.into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: None,
        name: name.into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: true,
        include_comments: Some(false),
    };
    let plan = publish_query::plan_query(&store, &request).unwrap();
    assert_eq!(plan.row_count, 1, "{name} retains its ordinary header row");
    let bundle = vec![("index.html".into(), b"<head></head>".to_vec())];
    let receipt = publish_query::publish_query_with_sheets_in(
        &store,
        &request,
        &plan.fingerprint,
        &bundle,
        vec![],
        scratch.path(),
    )
    .unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(std::path::Path::new(&receipt.path).join("app/snapshot.json")).unwrap(),
    )
    .unwrap();
    let query = snapshot["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["host"] == name)
        .unwrap();
    let groups = query["result"]["groups"].as_array().unwrap();
    assert_eq!(
        groups.len(),
        1,
        "Include the query text retains the selecting answer"
    );
    let blocks = groups[0]["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0][flag], true);
    assert!(blocks[0]["raw"].as_str().unwrap().contains(marker));
}

#[test]
fn comments_off_keeps_header_rows_as_query_sheet_inputs() {
    use tine_graph_features::publish::{sheet_export_inputs, SheetScope};
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::create_dir_all(scratch.path().join("pages")).unwrap();
    fs::write(
        scratch.path().join("pages/Header.md"),
        "status:: exported-state\n\n- Ordinary\n",
    )
    .unwrap();
    fs::write(
        scratch.path().join("pages/Preamble.md"),
        "Prose [[PreambleTarget]]\n\n- Ordinary\n",
    )
    .unwrap();
    fs::write(scratch.path().join("pages/Sheet.md"), "- {{query (property status exported-state)}}\n  tine.view:: table\n- {{query [[PreambleTarget]]}}\n  tine.view:: table\n").unwrap();
    let store = Store::open(scratch.path(), Default::default()).unwrap().0;
    let inputs = sheet_export_inputs(
        &store,
        Some(&["Sheet".into()]),
        Some(&SheetScope::Print {
            include_comments: Some(false),
        }),
    )
    .unwrap();
    assert_eq!(inputs.len(), 2);
    for (input, marker) in inputs.iter().zip(["exported-state", "Prose"]) {
        let query = input
            .query
            .as_ref()
            .expect("ordinary header query survives comment omission");
        assert_eq!(query.rows.len(), 1);
        assert!(query.rows[0].raw.contains(marker));
    }
}

#[test]
fn omitted_threads_never_become_sheet_inputs_or_query_sheet_rows() {
    use tine_graph_features::publish::{sheet_export_inputs, SheetScope};
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::create_dir_all(scratch.path().join("pages")).unwrap();
    fs::write(scratch.path().join("pages/Sheet.md"), "public:: true\n- Sheet\n  tine.view:: table\n  - TODO Body row\n  - TODO COMMENTMARKER\n    tine.quote:: Sheet\n    - TODO REPLYMARKER\n    - Hidden sheet\n      tine.view:: grid\n      - Cell\n- {{query (task TODO)}}\n  tine.view:: table\n").unwrap();
    let store = Store::open(scratch.path(), Default::default()).unwrap().0;
    let names = ["Sheet".to_owned()];
    for scope in [
        SheetScope::Live {
            all_pages: true,
            include_comments: Some(false),
        },
        SheetScope::Print {
            include_comments: Some(false),
        },
    ] {
        let inputs = sheet_export_inputs(&store, Some(&names), Some(&scope)).unwrap();
        assert_eq!(
            inputs.len(),
            2,
            "hidden sheet is excluded before evaluation"
        );
        assert_eq!(inputs[0].rows.len(), 1);
        let query = inputs[1].query.as_ref().unwrap();
        assert_eq!(query.rows.len(), 1);
        assert!(query.rows[0].raw.contains("Body row"));
        let serialized = serde_json::to_string(&inputs).unwrap();
        assert!(!serialized.contains("COMMENTMARKER"));
        assert!(!serialized.contains("REPLYMARKER"));
    }
}

#[test]
fn a_query_home_with_comments_off_contains_only_the_published_body_and_its_ids() {
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = scratch.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/Body.md"), "- TODO Body\n  - COMMENTMARKER\n    tine.quote:: Body\n    - REPLYMARKER\n  - TODO Nested body\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let request = publish_query::QueryExportRequest {
        argument: "(task TODO) {:remove-block-children? false}".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: None,
        name: "Body export".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: true,
        include_comments: Some(false),
    };
    let plan = publish_query::plan_query(&store, &request).unwrap();
    let bundle = vec![("index.html".into(), b"<head></head>".to_vec())];
    let receipt = publish_query::publish_query_with_sheets_in(
        &store,
        &request,
        &plan.fingerprint,
        &bundle,
        vec![],
        scratch.path(),
    )
    .unwrap();
    let snapshot =
        fs::read_to_string(std::path::Path::new(&receipt.path).join("app/snapshot.json")).unwrap();
    assert!(
        !snapshot.contains("COMMENTMARKER"),
        "baked query children must use the same comment projection as pages"
    );
    assert!(!snapshot.contains("REPLYMARKER"));
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    fn ids(blocks: &serde_json::Value, out: &mut Vec<String>) {
        for block in blocks.as_array().unwrap() {
            out.push(block["id"].as_str().unwrap().to_owned());
            ids(&block["children"], out);
        }
    }
    let mut published = Vec::new();
    for page in snapshot["pages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|page| page["path"] != "")
    {
        ids(&page["blocks"], &mut published);
    }
    let query = snapshot["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|query| query["host"] == "Body export")
        .unwrap();
    let mut rows = Vec::new();
    for group in query["result"]["groups"].as_array().unwrap() {
        ids(&group["blocks"], &mut rows);
    }
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|id| published.contains(id)), "each baked query row resolves to the published body after omission changes sibling positions");
}

#[test]
fn a_laid_out_sheet_still_exports_its_comments_as_annotations() {
    use tine_graph_features::{publish::sheet_export_inputs, SheetExport};
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::create_dir_all(scratch.path().join("pages")).unwrap();
    fs::write(scratch.path().join("pages/Sheet.md"), "- Sheet\n  tine.view:: table\n  - Body row\n    - Body cell\n    - CELLCOMMENT\n      tine.quote:: Body row\n  - COMMENTMARKER\n    tine.quote:: Sheet\n    - REPLYMARKER\n").unwrap();
    let store = Store::open(scratch.path(), Default::default()).unwrap().0;
    let inputs = sheet_export_inputs(&store, Some(&["Sheet".into()]), None).unwrap();
    let table = serde_json::json!({
        "page": "Sheet", "path": [0], "fp": inputs[0].fp, "query": false,
        "view": "table", "columns": [{"label": "Block", "formula": false}],
        "rows": [{"ix": 0, "title": "Body row", "bg": null, "cells": []}],
        "footer": null, "filterError": null, "omitted": 0,
    });
    let board = serde_json::json!({
        "page":"Sheet", "path":[0], "fp":inputs[0].fp, "view":"board",
        "columns":[{"label":"Rows", "cards":[{"ix":0, "title":"Body row", "bg":null,
            "chips":{"priority":"", "scheduled":"", "deadline":"", "tags":[]}}]}],
        "filterError":null, "omitted":0,
    });
    let grid = serde_json::json!({
        "page":"Sheet", "path":[0], "fp":inputs[0].fp, "view":"grid",
        "cols":1, "header":false, "footer":null, "omitted":0,
    });
    for data in [table, board, grid] {
        let view = data["view"].as_str().unwrap().to_owned();
        let sheet: SheetExport = serde_json::from_value(data).unwrap();
        let html =
            print::page_print_html_with_sheets(&store, "Sheet", Default::default(), vec![sheet])
                .unwrap()
                .unwrap();
        assert!(html.contains(&format!("class=\"sheet-{view}\"")));
        assert!(html.contains(if view == "grid" {
            "Body cell"
        } else {
            "Body row"
        }));
        assert!(html.contains("<aside class=\"comment\"><header>Comment</header>"));
        for text in ["COMMENTMARKER", "REPLYMARKER", "CELLCOMMENT"] {
            assert!(html.contains(text), "{view} lost {text}");
        }
    }
}

#[test]
fn query_sheet_titles_mark_both_comment_roots_and_unquoted_replies() {
    use tine_graph_features::{publish::sheet_export_inputs, SheetExport};
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::create_dir_all(scratch.path().join("pages")).unwrap();
    fs::write(scratch.path().join("pages/Sheet.md"), "- TODO Body row\n  - TODO COMMENTMARKER\n    tine.quote:: Body row\n    tine.author:: Ada\n    - TODO REPLYMARKER\n- {{query (task TODO) {:remove-block-children? false}}}\n  tine.view:: table\n").unwrap();
    let store = Store::open(scratch.path(), Default::default()).unwrap().0;
    let inputs = sheet_export_inputs(&store, Some(&["Sheet".into()]), None).unwrap();
    let query = inputs[0].query.as_ref().unwrap();
    assert_eq!(
        query.rows.len(),
        3,
        "the fixture selects the body, comment and reply as separate rows"
    );
    let rows: Vec<_> = query
        .rows
        .iter()
        .enumerate()
        .map(|(ix, row)| {
            serde_json::json!({
                "ix": ix, "title": row.raw.lines().next().unwrap(), "bg": null, "cells": [],
            })
        })
        .collect();
    for view in ["table", "board"] {
        let mut data = serde_json::json!({
            "page": "Sheet", "path": inputs[0].path, "fp": query.fp, "query": true,
            "view": view, "footer": null, "filterError": null, "omitted": 0,
        });
        if view == "table" {
            data["columns"] = serde_json::json!([{"label": "Block", "formula": false}]);
            data["rows"] = serde_json::json!(rows);
        } else {
            let cards: Vec<_> = rows.iter().map(|row| {
                let mut card = row.clone();
                card["chips"] = serde_json::json!({"priority":"", "scheduled":"", "deadline":"", "tags":[]});
                card
            }).collect();
            data["columns"] = serde_json::json!([{"label":"TODO", "cards":cards}]);
        }
        let sheet: SheetExport = serde_json::from_value(data).unwrap();
        let html =
            print::page_print_html_with_sheets(&store, "Sheet", Default::default(), vec![sheet])
                .unwrap()
                .unwrap();
        let rendered = html
            .split(&format!("class=\"sheet-{view}\""))
            .nth(1)
            .unwrap();
        assert!(rendered.contains("<header>Comment · Ada</header>"));
        assert!(rendered.contains("<header>Comment</header>"));
        assert!(rendered.contains("COMMENTMARKER"));
        assert!(rendered.contains("REPLYMARKER"));
        assert!(rendered.contains("Body row"));
    }
}

#[test]
fn comment_surface_inventory() {
    let _integration_guard = crate::integration_lock();
    let scratch = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = scratch.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    for (name, body) in [
        ("Markdown.md", "public:: true\n- Body passage\n  - COMMENTMARKER [[Target]]\n    tine.quote:: Body passage\n    - REPLYMARKER [[Target]]\n"),
        ("Org.org", "#+public: true\n* Body passage\n** COMMENTMARKER [[Target]]\n:PROPERTIES:\n:tine.quote: Body passage\n:END:\n*** REPLYMARKER [[Target]]\n"),
    ] {
        fs::write(root.join("pages").join(name), body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    let whole = store.whole_graph().unwrap();
    let search = whole
        .search(
            &SearchRequest {
                text: "COMMENTMARKER".into(),
                transliteration: false,
                within: None,
                page_limit: 10,
                block_limit: 10,
                explain: false,
                page_match_scope: Some(FriendlyPageMatchScope::Both),
                page_view: None,
                block_view: None,
            },
            &Cancel::default(),
        )
        .unwrap();
    let mut blocks = 0;
    let mut pages = 0;
    for hit in &search.hits {
        match hit {
            QueryHit::Block {
                block,
                display_text,
                ..
            } => {
                blocks += 1;
                assert!(display_text.contains("COMMENTMARKER"));
                assert!(!display_text.contains("tine.quote"));
                assert!(block.raw.contains("tine.quote"));
                assert_eq!(block.breadcrumb, ["Body passage"]);
            }
            QueryHit::Page { .. } => pages += 1,
        }
    }
    assert_eq!(
        (pages, blocks),
        (2, 2),
        "comments also make content-search page hits"
    );
    let backlinks = whole.backlinks("Target").unwrap();
    assert_eq!(backlinks.len(), 2);
    let (_, counts) = whole.page_backlink_counts();
    assert_eq!(
        counts.iter().find(|(name, _)| name == "Target").unwrap().1,
        4
    );
    for name in ["Markdown", "Org"] {
        let html = print::page_print_html(&store, name, Default::default())
            .unwrap()
            .unwrap();
        assert!(html.contains("COMMENTMARKER"));
        assert!(html.contains("REPLYMARKER"));
        assert!(html.contains("<aside class=\"comment\"><header>Comment</header>"));
        assert!(!html.contains("tine.quote"));
        let omitted = print::page_print_html(
            &store,
            name,
            print::PrintOpts {
                include_comments: false,
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(omitted.contains("Body passage"));
        assert!(!omitted.contains("COMMENTMARKER"));
        assert!(!omitted.contains("REPLYMARKER"));
    }
    let site = publish_query::publish_static(&store, scratch.path(), "site", true, true).unwrap();
    let mut html_pages = 0;
    for entry in fs::read_dir(&site.path).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "html") {
            let html = fs::read_to_string(path).unwrap();
            if html.contains("COMMENTMARKER") {
                html_pages += 1;
                assert!(html.contains("REPLYMARKER"));
                assert!(html.contains("<aside class=\"comment\"><header>Comment</header>"));
                assert!(!html.contains("tine.quote"));
            }
        }
    }
    assert_eq!(html_pages, 2);
    let index =
        fs::read_to_string(std::path::Path::new(&site.path).join("search-index.js")).unwrap();
    assert!(index.contains("COMMENTMARKER"));
    assert!(index.contains("REPLYMARKER"));
    let bundle = vec![("index.html".into(), b"<head></head>".to_vec())];
    let live = publish_query::publish_live_with_sheets_in(
        &store,
        scratch.path(),
        "live",
        true,
        true,
        &bundle,
        vec![],
        scratch.path(),
    )
    .unwrap();
    let snapshot =
        fs::read_to_string(std::path::Path::new(&live.path).join("app/snapshot.json")).unwrap();
    assert!(snapshot.contains("COMMENTMARKER"));
    assert!(snapshot.contains("REPLYMARKER"));
    assert!(snapshot.contains("tine.quote"));
    let request = publish_query::QueryExportRequest {
        argument: "@page and name = 'Markdown' or name = 'Org'".into(),
        dialect: QueryTextDialect::MacroTql,
        properties: vec![],
        current_page: None,
        name: "Comments".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
        include_comments: None,
    };
    let prepared =
        quick_share::prepare(&store, &request, "2026-11-13T00:00:00Z", scratch.path()).unwrap();
    let included_envelope = quick_share::seal(
        &prepared.payload,
        b"tine-quick-share-fixture-key-32b",
        b"fixture-share-id",
        b"fixture-nonc",
    );
    let bytes =
        quick_share::gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap();
    let document = String::from_utf8(bytes).unwrap();
    assert!(document.contains("COMMENTMARKER"));
    assert!(document.contains("REPLYMARKER"));
    assert!(
        document.contains("tine.quote"),
        "quick share retains source properties"
    );
    let mut without = request.clone();
    without.include_comments = Some(false);
    let with_plan = publish_query::plan_query(&store, &request).unwrap();
    let without_plan = publish_query::plan_query(&store, &without).unwrap();
    assert_ne!(with_plan.fingerprint, without_plan.fingerprint);
    assert!(!without_plan.include_comments);
    let prepared =
        quick_share::prepare(&store, &without, "2026-11-13T00:00:00Z", scratch.path()).unwrap();
    let omitted = String::from_utf8(
        quick_share::gunzip_bounded(&prepared.payload, quick_share::MAX_DOCUMENT_BYTES).unwrap(),
    )
    .unwrap();
    assert!(omitted.contains("Body passage"));
    assert!(!omitted.contains("COMMENTMARKER"));
    assert!(!omitted.contains("REPLYMARKER"));
    without.include_comments = None;
    without
        .properties
        .push(("tine.comments".into(), "false".into()));
    assert!(
        !publish_query::plan_query(&store, &without)
            .unwrap()
            .include_comments
    );
    for live in [false, true] {
        let receipt = if live {
            publish_query::publish_live_with_sheets_in(
                &store,
                scratch.path(),
                "without-live",
                true,
                false,
                &bundle,
                vec![],
                scratch.path(),
            )
            .unwrap()
        } else {
            publish_query::publish_static(&store, scratch.path(), "without-static", true, false)
                .unwrap()
        };
        let index = fs::read_to_string(std::path::Path::new(&receipt.path).join("search-index.js"))
            .unwrap();
        assert!(index.contains("Body passage"));
        assert!(!index.contains("COMMENTMARKER"));
        assert!(!index.contains("REPLYMARKER"));
        if live {
            let snapshot =
                fs::read_to_string(std::path::Path::new(&receipt.path).join("app/snapshot.json"))
                    .unwrap();
            assert!(snapshot.contains("Body passage"));
            assert!(!snapshot.contains("COMMENTMARKER"));
            assert!(!snapshot.contains("REPLYMARKER"));
        }
    }
    let plan = publish_query::plan_query(&store, &without).unwrap();
    let query = publish_query::publish_query_with_sheets_in(
        &store,
        &without,
        &plan.fingerprint,
        &bundle,
        vec![],
        scratch.path(),
    )
    .unwrap();
    let snapshot =
        fs::read_to_string(std::path::Path::new(&query.path).join("app/snapshot.json")).unwrap();
    assert!(snapshot.contains("Body passage"));
    assert!(!snapshot.contains("COMMENTMARKER"));
    assert!(!snapshot.contains("REPLYMARKER"));
    if let Some(out) = std::env::var_os("TINE_COMMENT_OUTPUT_ARTIFACTS") {
        let out = std::path::PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("live-omitted-snapshot.json"), &snapshot).unwrap();
        fs::write(out.join("share-document.json"), &document).unwrap();
        fs::write(out.join("share-envelope.txt"), &included_envelope).unwrap();
        let included =
            fs::read_to_string(std::path::Path::new(&live.path).join("app/snapshot.json")).unwrap();
        fs::write(out.join("live-included-snapshot.json"), included).unwrap();
    }
}
