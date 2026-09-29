//! og I1f (#35, port of master's `AppHome`): a live export opens on the page
//! the graph configures as `:default-home` when that page is exported, on an
//! explicitly requested page when one is named, and refuses a requested home
//! that is not exported before writing anything.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::publish_query::{publish_live, publish_live_home};
use tine_store::Store;

fn fixture(config: &str) -> (PathBuf, Store) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!(
        "tine-live-home-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let graph = base.join("graph");
    fs::create_dir_all(graph.join("pages")).unwrap();
    fs::create_dir_all(graph.join("journals")).unwrap();
    fs::create_dir_all(graph.join("logseq")).unwrap();
    fs::create_dir_all(base.join("output")).unwrap();
    fs::write(graph.join("logseq/config.edn"), config).unwrap();
    for page in ["Alpha", "Directory", "Hidden"] {
        let public = if page == "Hidden" {
            ""
        } else {
            "public:: true\n"
        };
        fs::write(
            graph.join(format!("pages/{page}.md")),
            format!("{public}- {page}\n"),
        )
        .unwrap();
    }
    let store = Store::open(&graph, Default::default()).unwrap().0;
    (base, store)
}

fn bundle() -> Vec<(String, Vec<u8>)> {
    vec![(
        "index.html".into(),
        b"<!doctype html><html><head><title>Tine</title></head><body></body></html>".to_vec(),
    )]
}

fn home(base: &std::path::Path, slug: &str) -> String {
    let snapshot: serde_json::Value = serde_json::from_slice(
        &fs::read(base.join("output").join(slug).join("app/snapshot.json")).unwrap(),
    )
    .unwrap();
    snapshot["home"].as_str().unwrap().to_owned()
}

#[test]
fn the_configured_home_opens_first_when_it_is_exported() {
    let (base, store) = fixture("{:default-home {:page \"directory\"}}\n");
    publish_live(&store, &base.join("output"), "Site", false, &bundle()).unwrap();
    assert_eq!(home(&base, "site"), "Directory");
}

#[test]
fn an_unexported_configured_home_falls_back_to_the_first_page() {
    let (base, store) = fixture("{:default-home {:page \"Hidden\"}}\n");
    publish_live(&store, &base.join("output"), "Site", false, &bundle()).unwrap();
    assert_eq!(home(&base, "site"), "Alpha");
}

#[test]
fn a_requested_home_wins_and_an_unexported_one_is_refused_before_writing() {
    let (base, store) = fixture("{:default-home {:page \"Directory\"}}\n");
    publish_live_home(
        &store,
        &base.join("output"),
        "Site",
        false,
        Some("alpha"),
        &bundle(),
    )
    .unwrap();
    assert_eq!(home(&base, "site"), "Alpha");

    let refused = publish_live_home(
        &store,
        &base.join("output"),
        "Other",
        false,
        Some("Hidden"),
        &bundle(),
    )
    .unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!base.join("output/other").exists());
}
