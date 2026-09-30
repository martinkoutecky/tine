use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_core::query::wire_parse::QueryTextDialect;
use tine_graph_features::publish_query::{
    plan_query, publish_live, publish_query, QueryExportRequest,
};
use tine_store::Store;

fn fixture() -> (PathBuf, PathBuf, Store) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!(
        "tine-query-publish-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let graph = base.join("graph");
    let output = base.join("output");
    fs::create_dir_all(graph.join("pages")).unwrap();
    fs::create_dir_all(graph.join("journals")).unwrap();
    fs::create_dir_all(&output).unwrap();
    fs::write(
        graph.join("pages/Public.md"),
        "public:: true\n- TODO selected\n",
    )
    .unwrap();
    fs::write(graph.join("pages/Secret.md"), "- DOING hidden\n").unwrap();
    let store = Store::open(&graph, Default::default()).unwrap().0;
    (graph, output, store)
}

fn bundle() -> Vec<(String, Vec<u8>)> {
    vec![("index.html".into(), b"<!doctype html><html><head><title>Tine</title></head><body><div id=\"root\"></div></body></html>".to_vec()),
         ("assets/app.js".into(), b"console.log('app')".to_vec())]
}

#[test]
fn query_publication_reviews_owner_pages_and_rejects_a_stale_plan() {
    let (graph, _output, store) = fixture();
    let request = QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: Some("Public".into()),
        name: "My TODOs".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
    };
    let plan = plan_query(&store, &request).unwrap();
    assert_eq!(plan.anchor, "block");
    assert_eq!(plan.row_count, 1);
    assert_eq!(plan.pages.len(), 1);
    assert_eq!(plan.pages[0].name, "Public");
    assert!(!graph.join("published-queries").exists());

    fs::write(
        graph.join("pages/Public.md"),
        "public:: true\n- TODO changed\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    assert!(publish_query(&store, &request, &plan.fingerprint, &bundle()).is_err());
    assert!(!graph.join("published-queries/my-todos").exists());

    let fresh = plan_query(&store, &request).unwrap();
    let receipt = publish_query(&store, &request, &fresh.fingerprint, &bundle()).unwrap();
    assert_eq!(receipt.pages, 1);
    assert!(graph
        .join("published-queries/my-todos/public.html")
        .exists());
    assert!(
        fs::read_to_string(graph.join("published-queries/my-todos/app/index.html"))
            .unwrap()
            .contains("tine-published")
    );
    let snapshot: serde_json::Value = serde_json::from_slice(
        &fs::read(graph.join("published-queries/my-todos/app/snapshot.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot["pages"].as_array().unwrap().len(), 2); // synthetic query home + selected owner
    assert_eq!(snapshot["queries"].as_array().unwrap().len(), 1);
    assert!(
        !fs::read_to_string(graph.join("published-queries/my-todos/pages.html"))
            .unwrap()
            .contains("DOING hidden")
    );
    assert!(!graph.join("publish").exists());
    store.close();
}

#[test]
fn live_publication_is_selection_closed_and_only_uses_a_picked_external_folder() {
    let (graph, output, store) = fixture();
    let public = publish_live(&store, &output, "Public site", false, &bundle()).unwrap();
    assert_eq!(public.pages, 1);
    let snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("public-site/app/snapshot.json")).unwrap())
            .unwrap();
    assert_eq!(snapshot["pages"].as_array().unwrap().len(), 1);
    assert!(!snapshot.to_string().contains("DOING hidden"));
    assert!(publish_live(&store, &output, "Public site", false, &bundle()).is_err());
    assert!(publish_live(&store, &graph, "inside", true, &bundle()).is_err());
    assert!(!graph.join("inside").exists());
    let all = publish_live(&store, &output, "All pages", true, &bundle()).unwrap();
    assert_eq!(all.pages, 2);
    store.close();
}

#[test]
fn live_snapshot_bakes_queries_on_selected_pages_and_closes_their_rows() {
    let (graph, output, store) = fixture();
    fs::write(
        graph.join("pages/Public.md"),
        "public:: true\n- TODO selected\n- {{query (task DOING)}}\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    publish_live(&store, &output, "Dashboard", false, &bundle()).unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("dashboard/app/snapshot.json")).unwrap())
            .unwrap();
    assert_eq!(snapshot["queries"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["queries"][0]["argument"], "(task DOING)");
    assert_eq!(snapshot["queries"][0]["result"]["total"], 0);
    assert!(!snapshot.to_string().contains("DOING hidden"));
    store.close();
}

/// E1: even the static fallback must not disclose how many matches live on
/// pages outside a query export. Exercise both page and block query renderers.
#[test]
fn query_export_nested_queries_do_not_disclose_outside_match_counts() {
    let (graph, _output, store) = fixture();
    fs::write(
        graph.join("pages/Public.md"),
        "public:: true\n- TODO selected\n- {{query (task DOING)}}\n- {{tine-query @page}}\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    let request = QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: None,
        name: "Nested queries".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
    };
    let plan = plan_query(&store, &request).unwrap();
    let receipt = publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap();
    let html = fs::read_to_string(PathBuf::from(receipt.path).join("public.html")).unwrap();
    assert!(html.contains("No matching blocks."));
    assert!(html.contains("query-count\">1</span>"));
    assert!(
        !html.contains("query-omitted"),
        "I-4/E1: query exports disclose only selected results; exemplar render_query_with_title"
    );
    assert!(!html.contains("DOING hidden"));
    tine_graph_features::publish::publish_html(&store).unwrap();
    let graph_html = fs::read_to_string(graph.join("publish/public.html")).unwrap();
    assert!(graph_html.contains("1 result on non-public pages omitted."));
    store.close();
}

#[test]
fn static_fallback_renders_tql_page_rows_from_the_ir_answerer() {
    let (graph, output, store) = fixture();
    fs::write(
        graph.join("pages/Public.md"),
        "public:: true\n- {{tine-query @page}}\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    publish_live(&store, &output, "Page query", false, &bundle()).unwrap();
    let html = fs::read_to_string(output.join("page-query/public.html")).unwrap();
    assert!(html.contains("query-count\">1</span>"), "{html}");
    assert!(!html.contains("secret.html"));
    store.close();
}

#[test]
fn selected_static_page_keeps_outside_references_inert() {
    let (graph, _output, store) = fixture();
    fs::write(
        graph.join("pages/Public.md"),
        "public:: true\n- TODO selected [[Secret]] and #secret\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    let request = QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: None,
        name: "Selected".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
    };
    let plan = plan_query(&store, &request).unwrap();
    publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap();
    let html = fs::read_to_string(graph.join("published-queries/selected/public.html")).unwrap();
    assert!(html.contains("ref-outside"), "{html}");
    assert!(html.contains("tag tag-outside"), "{html}");
    assert!(!html.contains("href=\"secret.html\""), "{html}");
    store.close();
}

#[test]
fn live_export_artifact_cost_is_bounded_per_selected_block() {
    fn bytes_under(path: &std::path::Path) -> u64 {
        fs::read_dir(path)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    bytes_under(&path)
                } else {
                    fs::metadata(path).unwrap().len()
                }
            })
            .sum()
    }
    let (graph, output, store) = fixture();
    for (count, name) in [(1, "one"), (60, "sixty")] {
        let source = format!("public:: true\n{}", "- TODO selected\n".repeat(count));
        fs::write(graph.join("pages/Public.md"), source).unwrap();
        store.scan_refresh().unwrap();
        publish_live(&store, &output, name, false, &bundle()).unwrap();
        let request = QueryExportRequest {
            argument: "(task TODO)".into(),
            dialect: QueryTextDialect::MacroQuery,
            properties: vec![],
            current_page: None,
            name: format!("Query {name}"),
            host_block_id: None,
            folder: None,
            replace: false,
            asset_budget_bytes: None,
        };
        let plan = plan_query(&store, &request).unwrap();
        let receipt = publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap();
        let bytes = bytes_under(&PathBuf::from(&receipt.path));
        eprintln!(
            "query unit cost: blocks={count} bytes={bytes} files={}",
            receipt.files
        );
        assert_eq!(receipt.pages, 1);
    }
    let one = bytes_under(&output.join("one"));
    let sixty = bytes_under(&output.join("sixty"));
    let one_snap = fs::metadata(output.join("one/app/snapshot.json"))
        .unwrap()
        .len();
    let sixty_snap = fs::metadata(output.join("sixty/app/snapshot.json"))
        .unwrap()
        .len();
    eprintln!("unit cost: full 1={one} 60={sixty}; snapshot 1={one_snap} 60={sixty_snap}");
    assert!(sixty > one && sixty - one < 100_000);
    assert!(sixty_snap > one_snap && sixty_snap - one_snap < 50_000);
    store.close();
}

#[test]
#[ignore = "run with TINE_QUERY_E2E_PARENT and a built dist/ for the browser smoke"]
fn build_real_query_site_for_browser_smoke() {
    let parent = PathBuf::from(std::env::var("TINE_QUERY_E2E_PARENT").unwrap());
    fs::create_dir_all(&parent).unwrap();
    let graph_path = std::env::var("TINE_QUERY_E2E_GRAPH").ok();
    let store = if let Some(path) = &graph_path {
        Store::open(&PathBuf::from(path), Default::default())
            .unwrap()
            .0
    } else {
        fixture().2
    };
    let request = QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: graph_path.is_none().then(|| "Public".into()),
        name: "Selected tasks".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
    };
    let plan = plan_query(&store, &request).unwrap();
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dist");
    let mut built = vec![(
        "index.html".into(),
        fs::read(dist.join("index.html")).unwrap(),
    )];
    for file in fs::read_dir(dist.join("assets")).unwrap() {
        let file = file.unwrap();
        if file.file_type().unwrap().is_file() {
            built.push((
                format!("assets/{}", file.file_name().to_string_lossy()),
                fs::read(file.path()).unwrap(),
            ));
        }
    }
    let receipt = publish_query(&store, &request, &plan.fingerprint, &built).unwrap();
    println!("query browser fixture: {}", receipt.path);
    store.close();
}

/// I-22 (og C3 L02): the macro scanner and the TQL prepass sliced block text
/// at byte offsets, so a multibyte character at the probed offset panicked
/// the whole publication (`publish_live` from the CLI aborts). Every one of
/// these is ordinary text a user can type; none may abort the export.
#[test]
fn live_publication_survives_multibyte_text_at_every_probed_offset() {
    let (graph, output, store) = fixture();
    fs::write(
        graph.join("pages/Public.md"),
        concat!(
            "public:: true\n",
            "- {{中文}}\n",
            "- {{中文}} {{query (task TODO)}}\n",
            "- {{ 名前 }}\n",
            "- {{ščř\n",
            "- {{query a}é}\n",
            "- {{tine-query @page ab中}}\n",
            "- {{tine-query @block žž}}\n",
            "- {{tine-query @block and #x\n  -- ab中}}\n",
            "- {{tine-query @block and #x\n  -- oř}}\n",
        ),
    )
    .unwrap();
    store.scan_refresh().unwrap();
    publish_live(&store, &output, "Multibyte", false, &bundle()).unwrap();
    assert!(output.join("multibyte/app/snapshot.json").is_file());
    store.close();
}

/// GH #560 (master 350efef1f): "0 pages exported" is the ordinary outcome for
/// a graph with no `public:: true` page; nothing non-public reaches the export.
#[test]
fn live_publication_of_a_graph_without_public_pages_exports_nothing() {
    let (graph, output, store) = fixture();
    fs::write(graph.join("pages/Public.md"), "- alpha body\n").unwrap();
    store.scan_refresh().unwrap();
    let receipt = publish_live(&store, &output, "Nothing public", false, &bundle()).unwrap();
    assert_eq!(
        receipt.pages, 0,
        "publication is the public-page capability"
    );
    let mut stack = vec![output.join("nothing-public")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let text = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
                assert!(
                    !text.contains("alpha body") && !text.contains("DOING hidden"),
                    "a non-public page reached {}",
                    path.display()
                );
            }
        }
    }
    store.close();
}

#[test]
fn query_export_uses_the_graph_leaf_and_reports_missing_assets() {
    let (graph, _output, store) = fixture();
    fs::write(
        graph.join("pages/Public.md"),
        "- TODO ![missing](../assets/missing.png)\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    let request = QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: None,
        name: "Portable".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
    };
    let plan = plan_query(&store, &request).unwrap();
    let receipt = publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap();
    // The receipt reports the Store's resolved root, as master does (verbatim `\\?\` form on Windows).
    assert_eq!(
        fs::canonicalize(&receipt.path).unwrap(),
        fs::canonicalize(graph.join("published-queries/portable")).unwrap(),
        "I-12: the query action commits through Store into its graph output leaf"
    );
    let wire = serde_json::to_value(&receipt).unwrap();
    assert!(
        wire["warnings"].as_array().is_some_and(|warnings| warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("missing.png"))),
        "I-4: omitted assets must be reported before a leaf is moved"
    );
    store.close();
}

#[test]
fn query_export_default_budget_accepts_assets_above_the_old_ceiling() {
    let (graph, _output, store) = fixture();
    fs::create_dir_all(graph.join("assets")).unwrap();
    fs::File::create(graph.join("assets/video.mp4"))
        .unwrap()
        .set_len(33 * 1024 * 1024)
        .unwrap();
    fs::write(
        graph.join("pages/Public.md"),
        "- TODO [video](../assets/video.mp4)\n",
    )
    .unwrap();
    store.scan_refresh().unwrap();
    let request = QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: None,
        name: "Video".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
    };
    let plan = plan_query(&store, &request).unwrap();
    let result = publish_query(&store, &request, &plan.fingerprint, &bundle());
    assert!(
        result.is_ok(),
        "the 1 GiB query budget replaces the fixed 32 MiB ceiling: {result:?}"
    );
    store.close();
}

#[test]
fn query_leaf_assets_survive_a_move_and_budget_refusal_leaves_old_output() {
    let (graph, output, store) = fixture();
    for (path, bytes) in [
        ("left/pic.png", b"left".as_slice()),
        ("right/pic.png", b"right".as_slice()),
        ("notes.pdf", b"pdf".as_slice()),
        ("clip.ogg", b"audio".as_slice()),
        ("space name.png", b"space".as_slice()),
    ] {
        let path = graph.join("assets").join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let text = "- TODO assets\n  ![left](../assets/left/pic.png) ![right](../assets/right/pic.png)\n  [PDF](../assets/notes.pdf) [audio](../assets/clip.ogg) ![space](../assets/space%20name.png)\n";
    fs::write(graph.join("pages/Public.md"), text).unwrap();
    store.scan_refresh().unwrap();
    let mut request = QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: None,
        name: "Assets".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
    };
    let plan = plan_query(&store, &request).unwrap();
    let first = publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap();
    let old_html = fs::read(PathBuf::from(&first.path).join("public.html")).unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    request.replace = true;
    request.asset_budget_bytes = Some(4);
    let error = publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap_err();
    assert!(
        error
            .get_ref()
            .unwrap()
            .is::<tine_graph_features::publish_query::AssetBudgetExceeded>(),
        "typed refusal is the UI action boundary"
    );
    assert_eq!(
        fs::read(PathBuf::from(&first.path).join("public.html")).unwrap(),
        old_html
    );
    assert!(
        !graph.join("logseq/.tine-trash/conflicts").exists(),
        "asset budget fails before retirement"
    );
    request.asset_budget_bytes = None;
    let replacement = publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap();
    assert_eq!(
        fs::read(PathBuf::from(replacement.retired.unwrap()).join("public.html")).unwrap(),
        old_html
    );
    let separate = plan_query(&store, &request)
        .unwrap()
        .suggested_folder
        .unwrap();
    request.folder = Some(separate);
    request.replace = false;
    let second = publish_query(&store, &request, &plan.fingerprint, &bundle()).unwrap();
    fs::rename(&second.path, output.join("moved")).unwrap();
    for (path, expected) in [
        ("left/pic.png", "left"),
        ("right/pic.png", "right"),
        ("notes.pdf", "pdf"),
        ("clip.ogg", "audio"),
        ("space name.png", "space"),
    ] {
        assert_eq!(
            fs::read(output.join("moved/assets").join(path)).unwrap(),
            expected.as_bytes()
        );
    }
    let html = fs::read_to_string(output.join("moved/public.html")).unwrap();
    assert!(!html.contains("../assets/"));
    assert!(html.contains("assets/left/pic.png"));
    assert!(html.contains("assets/notes.pdf"));
    assert!(html.contains("assets/clip.ogg"));
    assert_eq!(
        fs::read(graph.join("pages/Public.md")).unwrap(),
        text.as_bytes()
    );
    store.close();
}
