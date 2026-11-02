//! og-perf-rename-walks (cost-model audit 2026-11-02 row 5): a rename, a
//! page delete and a rescue (rename file to page) walk the graph's text files
//! once per operation, never its assets. Before, a rename stat-walked every
//! page file and every asset twice (plan and plan reuse, once more per
//! replan), a delete and each rescue try once with the assets. The one walk
//! left is the plan's only proof that it sees an external edit the watcher
//! has not published yet (`pages::refreshed_view`); both tests pin it.
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tine_core::model::PageKind;
use tine_graph_features::pages;
use tine_store::cost_counters::{self, Counts};
use tine_store::{PageHost, Store};

const PAGES: usize = 120;
const ASSETS: usize = 80;

struct Graph {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    store: Arc<Store>,
    host: PageHost,
    /// Graph-text files on disk when the operation starts.
    text_files: u64,
}

fn graph(watch: tine_store::WatchMode) -> Graph {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("graph");
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for index in 0..PAGES {
        fs::write(
            root.join(format!("pages/Other{index:04}.md")),
            format!("- unrelated {index}\n"),
        )
        .unwrap();
    }
    for index in 0..ASSETS {
        fs::write(root.join(format!("assets/image{index:04}.png")), b"png").unwrap();
    }
    fs::write(root.join("pages/Target.md"), "- the target\n").unwrap();
    fs::write(root.join("pages/Ref.md"), "- links [[Target]]\n").unwrap();
    fs::write(root.join("journals/Loose.md"), "- a stray file\n").unwrap();
    let options = tine_store::OpenOptions {
        watch,
        ..Default::default()
    };
    let store = Arc::new(Store::open(&root, options).unwrap().0);
    store.whole_graph().unwrap();
    let host = PageHost::start_for_tests(&store, &temp.path().join("app")).unwrap();
    assert!(host.quiesce_for_tests(Duration::from_secs(30)));
    Graph {
        _temp: temp,
        root,
        store,
        host,
        text_files: PAGES as u64 + 3,
    }
}

fn measured(graph: &Graph, operation: impl FnOnce()) -> Counts {
    cost_counters::reset();
    operation();
    let counts = cost_counters::snapshot();
    assert!(graph.host.quiesce_for_tests(Duration::from_secs(30)));
    counts
}

fn one_text_walk(what: &str, graph: &Graph, counts: Counts) {
    assert_eq!(
        counts.listing_stamps, graph.text_files,
        "{what}: exactly one stat walk of the graph's text files per operation, not one \
         per plan, and not none: it is the only proof against an unpublished external \
         edit (og-perf-rename-walks; exemplar pages.rs refreshed_view): {counts:?}"
    );
    assert_eq!(
        counts.asset_listing_stamps, 0,
        "{what}: no page plan reads assets, so it walks none (Depth::PageStamps): {counts:?}"
    );
}

#[test]
fn a_page_operation_walks_the_graph_text_once_and_no_asset() {
    let _integration_guard = crate::integration_lock();
    let graph = graph(Default::default());
    let renamed = measured(&graph, || {
        pages::rename_or_merge_page(
            &graph.store,
            &graph.host,
            "Target",
            "Renamed",
            None,
            None,
            &[],
        )
        .unwrap();
    });
    assert_eq!(
        fs::read_to_string(graph.root.join("pages/Ref.md")).unwrap(),
        "- links [[Renamed]]\n"
    );
    one_text_walk("rename", &graph, renamed);

    let rescued = measured(&graph, || {
        pages::rename_file_to_page(&graph.store, &graph.host, "journals/Loose.md", "Rescued")
            .unwrap();
    });
    assert!(graph.root.join("pages/Rescued.md").is_file());
    one_text_walk("rename file to page", &graph, rescued);

    let session = serde_json::to_value(graph.host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    let deleted = measured(&graph, || {
        pages::delete_page_expected(
            &graph.store,
            &graph.host,
            session,
            "Other0001",
            PageKind::Page,
            Some("pages/Other0001.md"),
        )
        .unwrap();
    });
    assert!(!graph.root.join("pages/Other0001.md").exists());
    one_text_walk("delete", &graph, deleted);
    graph.store.close();
}

/// The walk the operation keeps: a referrer an external editor or a sync
/// client wrote just before the rename, which the watcher has not published,
/// is still rewritten. The watcher is stopped, a watch that installs but
/// never reports (some network and FUSE mounts), so only the operation's
/// walk can see `Late.md`.
#[test]
fn a_rename_sees_a_referrer_the_watcher_has_not_published() {
    let _integration_guard = crate::integration_lock();
    let graph = graph(Default::default());
    graph.store.stop_watcher_for_tests();
    let published = graph.store.whole_graph().unwrap().rev();
    fs::write(graph.root.join("pages/Late.md"), "- late [[Target]]\n").unwrap();
    assert_eq!(graph.store.whole_graph().unwrap().rev(), published);
    pages::rename_or_merge_page(
        &graph.store,
        &graph.host,
        "Target",
        "Renamed",
        None,
        None,
        &[],
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(graph.root.join("pages/Late.md")).unwrap(),
        "- late [[Renamed]]\n"
    );
    // No quiesce: the host's settling waits on the stopped watcher.
    graph.store.close();
}
