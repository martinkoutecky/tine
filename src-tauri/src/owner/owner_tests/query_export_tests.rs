use super::*;

const QUERY_ID: &str = "ad674ffe-93e3-4cbf-873e-548d83f69b0f";
static EXPORT_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn query_graph() -> Graph {
    Graph::seeded(
        tine_store::WatchMode::Poll,
        &[
            (
                "pages/Query.md",
                &format!("- {{{{query (task TODO)}}}}\n  id:: {QUERY_ID}\n"),
            ),
            ("pages/Selected.md", "- TODO selected\n"),
        ],
    )
}

fn output_files(path: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, dir: &Path, files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = Default::default();
    collect(path, path, &mut files);
    files
}

#[test]
fn warm_owner_query_export_matches_shared_publisher_without_source_reopen() {
    let _guard = EXPORT_TEST.lock().unwrap();
    let graph = query_graph();
    graph.store.stop_watcher_for_tests();
    let args = || super::super::query_export::Args {
        uuid: QUERY_ID.into(),
        name: Some("Review".into()),
        force: false,
        no_comments: false,
        epoch: 0,
    };
    // Freeze the old path's shared publisher output before using the socket.
    let expected =
        super::super::query_export::publish(&graph.store, args(), &crate::cli::bundle()).unwrap();
    let expected_files = output_files(Path::new(&expected.path));
    let slot = Arc::new(graph.slot());
    let gate = Gate::new();
    let _endpoint = Endpoint::bind(
        &graph.paths().socket,
        hello(&graph.root, "app"),
        gate.clone(),
        Arc::new(Owner(slot)),
    )
    .unwrap();
    let mut conn = graph.connect();
    assert_eq!(
        code(conn.call("query.export", json!({"uuid":QUERY_ID,"force":"invalid"}))),
        "bad-args"
    );
    tine_store::cost_counters::take_query_export_work(&graph.root);
    tine_store::cost_counters::take_query_uuid_blocks(QUERY_ID);
    let receipt = conn
        .call(
            "query.export",
            json!({"uuid":QUERY_ID, "name":"Review", "force":true, "epoch":0}),
        )
        .unwrap();
    assert_eq!(receipt["pages"], 1);
    assert_eq!(
        output_files(Path::new(receipt["path"].as_str().unwrap())),
        expected_files
    );
    assert_eq!(
        tine_store::cost_counters::take_query_export_work(&graph.root).opens,
        0
    );
    assert_eq!(
        tine_store::cost_counters::take_query_uuid_blocks(QUERY_ID),
        (1, 0)
    );
    let refused = conn.call(
        "query.export",
        json!({"uuid":QUERY_ID, "name":"Review", "force":false, "epoch":0}),
    );
    match refused {
        Err(Failure::Owner(body)) => assert!(body.message.contains("use --force")),
        other => panic!("{other:?}"),
    }
    gate.pause();
    assert_eq!(
        code(conn.call(
            "query.export",
            json!({"uuid":QUERY_ID,"force":true,"epoch":0})
        )),
        "busy"
    );
}

#[test]
fn cold_store_owner_query_export_loads_once() {
    let _guard = EXPORT_TEST.lock().unwrap();
    let graph = query_graph();
    graph.store.close();
    tine_store::cost_counters::take_query_export_work(&graph.root);
    let store = Arc::new(Store::open(&graph.root, Default::default()).unwrap().0);
    let mut slot = GraphSlot::new(store, graph.root.clone());
    *slot.host.get_mut().unwrap() =
        PageHostSlot::Running(PageHost::start_for_tests(&slot.store, &graph.app_data).unwrap());
    let (_endpoint, _gate) = endpoint(&graph, Arc::new(slot));
    let mut conn = graph.connect();
    let receipt = conn
        .call(
            "query.export",
            json!({"uuid":QUERY_ID,"force":false,"epoch":86400}),
        )
        .unwrap();
    assert_eq!(receipt["pages"], 1);
    // The invoking CLI's clock stamps the output, not the owner's own
    // environment or time (review hold 4aa451d71: `SOURCE_DATE_EPOCH=… tine
    // export query` against a running owner).
    let snapshot: serde_json::Value = serde_json::from_slice(
        &std::fs::read(Path::new(receipt["path"].as_str().unwrap()).join("app/snapshot.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot["exported_at"], "1970-01-02T00:00:00Z");
    assert_eq!(
        tine_store::cost_counters::take_query_export_work(&graph.root).opens,
        1
    );
}
