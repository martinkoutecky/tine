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
    let (graph, output, store) = fixture();
    let request = QueryExportRequest {
        argument: "(task TODO)".into(),
        dialect: QueryTextDialect::MacroQuery,
        properties: vec![],
        current_page: Some("Public".into()),
        name: "My TODOs".into(),
        host_block_id: None,
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
    assert!(publish_query(&store, &request, &plan.fingerprint, &output, &bundle()).is_err());
    assert!(!output.join("my-todos").exists());

    let fresh = plan_query(&store, &request).unwrap();
    let receipt = publish_query(&store, &request, &fresh.fingerprint, &output, &bundle()).unwrap();
    assert_eq!(receipt.pages, 1);
    assert!(output.join("my-todos/public.html").exists());
    assert!(fs::read_to_string(output.join("my-todos/app/index.html"))
        .unwrap()
        .contains("tine-published"));
    let snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("my-todos/app/snapshot.json")).unwrap())
            .unwrap();
    assert_eq!(snapshot["pages"].as_array().unwrap().len(), 2); // synthetic query home + selected owner
    assert_eq!(snapshot["queries"].as_array().unwrap().len(), 1);
    assert!(!fs::read_to_string(output.join("my-todos/pages.html"))
        .unwrap()
        .contains("DOING hidden"));
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
    let (graph, output, store) = fixture();
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
    };
    let plan = plan_query(&store, &request).unwrap();
    publish_query(&store, &request, &plan.fingerprint, &output, &bundle()).unwrap();
    let html = fs::read_to_string(output.join("selected/public.html")).unwrap();
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
    let receipt = publish_query(&store, &request, &plan.fingerprint, &parent, &built).unwrap();
    println!("query browser fixture: {}", receipt.path);
    store.close();
}
