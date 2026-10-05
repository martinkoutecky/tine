use std::{
    fs,
    path::Path,
    process::{Child, Command},
    time::{Duration, Instant},
};
use tine_store::{publish::publish_query_site, Store};

fn open(root: &Path) -> Store {
    Store::open(root, Default::default()).unwrap().0
}
fn old(root: &Path) {
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/source.md"), "- source\n").unwrap();
    fs::create_dir_all(root.join("published-queries/tasks")).unwrap();
    fs::write(root.join("published-queries/tasks/index.html"), b"old").unwrap();
}
fn recoveries(root: &Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(root.join("logseq/.tine-trash/conflicts"))
        .unwrap()
        .map(|e| e.unwrap().path().join("previous"))
        .collect()
}
#[test]
fn replace_reports_the_leaf_that_arrived_after_review_and_leaves_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    old(root);
    fs::create_dir_all(root.join("published-queries/sibling")).unwrap();
    fs::write(
        root.join("published-queries/sibling/index.html"),
        b"sibling",
    )
    .unwrap();
    let store = open(root);
    let review = tine_store::publish::query_publication_destination(&store, "tasks").unwrap();
    assert!(review.1);
    assert_eq!(review.2.as_deref(), Some("tasks-2"));
    let result = publish_query_site(&store, "tasks", true, &mut |w| {
        fs::write(root.join("published-queries/tasks/index.html"), b"arrived").unwrap();
        w.write("index.html", b"new")
    })
    .unwrap();
    assert_eq!(
        fs::read(result.previous_kept.unwrap().join("index.html")).unwrap(),
        b"arrived"
    );
    assert_eq!(
        fs::read(root.join("published-queries/sibling/index.html")).unwrap(),
        b"sibling"
    );
    assert_eq!(fs::read(result.site.join("index.html")).unwrap(), b"new");
    store.scan_refresh().unwrap();
    assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), 1);
    store.close();
}

#[test]
fn publication_child() {
    let Ok(root) = std::env::var("TINE_QUERY_COMMIT_CHILD") else {
        return;
    };
    let root = Path::new(&root);
    let store = open(root);
    let replace = std::env::var("TINE_QUERY_REPLACE").as_deref() == Ok("yes");
    let result = publish_query_site(&store, "tasks", replace, &mut |w| {
        w.write("index.html", b"new")
    });
    match result {
        Ok(_) => fs::write(root.join("result"), b"success").unwrap(),
        Err(error) => fs::write(
            root.join("result"),
            format!(
                "{:?}:{}",
                error.cause.kind,
                error.previous_kept.unwrap_or_default().display()
            ),
        )
        .unwrap(),
    }
    store.close();
}
fn child(root: &Path, point: &str, replace: bool) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "publication_child", "--nocapture"])
        .env("TINE_QUERY_COMMIT_CHILD", root)
        .env("TINE_QUERY_REPLACE", if replace { "yes" } else { "no" })
        .env("TINE_PUBLICATION_PAUSE", point)
        .env("TINE_PUBLICATION_MARKER", root.join("paused"))
        .spawn()
        .unwrap()
}
fn paused(root: &Path, child: &mut Child) {
    let start = Instant::now();
    while !fs::read_dir(root.join("published-queries")).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".tine-publish-stage-")
                && fs::read(entry.path().join("index.html")).is_ok_and(|bytes| bytes == b"new")
        })
    }) {
        if start.elapsed() > Duration::from_secs(15) {
            child.kill().unwrap();
            panic!("publication child did not reach the commit boundary");
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before boundary"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn killed_replace_reopens_with_the_complete_previous_leaf_in_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    old(root);
    let mut proc = child(root, "retired", true);
    paused(root, &mut proc);
    let start = Instant::now();
    while root.join("published-queries/tasks").exists() {
        assert!(start.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(5));
    }
    proc.kill().unwrap();
    proc.wait().unwrap();
    assert!(!root.join("published-queries/tasks").exists());
    let kept = recoveries(root);
    assert_eq!(kept.len(), 1);
    assert_eq!(fs::read(kept[0].join("index.html")).unwrap(), b"old");
    let store = open(root);
    assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), 1);
    let result = publish_query_site(&store, "tasks", false, &mut |w| {
        w.write("index.html", b"complete")
    })
    .unwrap();
    assert_eq!(
        fs::read(result.site.join("index.html")).unwrap(),
        b"complete"
    );
    assert_eq!(fs::read(kept[0].join("index.html")).unwrap(), b"old");
    store.close();
}
#[test]
fn concurrent_create_and_replace_winners_are_never_clobbered() {
    for replace in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        if replace {
            old(root);
        } else {
            fs::create_dir_all(root.join("pages")).unwrap();
        }
        let mut proc = child(root, "before-install", replace);
        paused(root, &mut proc);
        if replace {
            let start = Instant::now();
            while root.join("published-queries/tasks").exists() {
                assert!(start.elapsed() < Duration::from_secs(15));
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        fs::create_dir_all(root.join("published-queries/tasks")).unwrap();
        fs::write(root.join("published-queries/tasks/index.html"), b"winner").unwrap();
        fs::write(root.join("paused.continue"), b"go").unwrap();
        assert!(proc.wait().unwrap().success());
        assert_eq!(
            fs::read(root.join("published-queries/tasks/index.html")).unwrap(),
            b"winner"
        );
        let result = fs::read_to_string(root.join("result")).unwrap();
        assert!(result.starts_with("AlreadyExists:"));
        if replace {
            let path = result.strip_prefix("AlreadyExists:").unwrap();
            assert_eq!(
                fs::read(Path::new(path).join("index.html")).unwrap(),
                b"old"
            );
        }
    }
}

#[test]
fn graph_site_and_query_leaf_have_one_commit_answerer() {
    let source = include_str!("../src/publish.rs");
    let production = source.split("#[cfg(all(test, unix))]").next().unwrap();
    // Count CALLS, whatever the receiver or argument spelling: every occurrence of the
    // name followed by `(` that is not its own `fn` definition.
    let calls = |name: &str| {
        let squeezed: String = production.split_whitespace().collect::<Vec<_>>().join(" ");
        squeezed.matches(&format!("{name}(")).count()
            - squeezed.matches(&format!("fn {name}(")).count()
    };
    assert_eq!(calls("commit_publish_stage_report"), 1,
        "I-12: all graph publications share one stage/commit door; exemplar publish.rs::publish_site_at");
    // Both public doors (graph site, query leaf) reach that one answerer through publish_site_at.
    assert_eq!(
        calls("publish_site_at"),
        2,
        "I-12: the graph and query-leaf publications both go through publish_site_at"
    );
    assert_eq!(calls("collect_asset_refs"), 1,
        "I-12: publication reuses the asset-reference answerer; exemplar publish.rs::publication_assets");
    let contract = include_str!("../../../docs/storage-contract.md");
    for value in [
        "published-queries/<portable-folder>/",
        "1 GiB",
        "AssetBudgetExceeded",
        "Windows",
        "previous-publish/previous",
    ] {
        assert!(
            contract.contains(value),
            "publication contract drift: {value}"
        );
    }
}

#[test]
fn review_suggestions_remain_portable_at_the_folder_limit() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    let folder = "a".repeat(80);
    fs::create_dir_all(dir.path().join("published-queries").join(&folder)).unwrap();
    let store = open(dir.path());
    let (_, exists, suggestion) =
        tine_store::publish::query_publication_destination(&store, &folder).unwrap();
    assert!(exists);
    let suggestion = suggestion.unwrap();
    assert!(suggestion.len() <= 80);
    publish_query_site(&store, &suggestion, false, &mut |w| {
        w.write("index.html", b"new")
    })
    .unwrap();
    store.close();
}

#[cfg(unix)]
#[test]
fn review_refuses_an_unsafe_existing_leaf() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::create_dir_all(dir.path().join("published-queries")).unwrap();
    std::os::unix::fs::symlink(
        dir.path().join("pages"),
        dir.path().join("published-queries/tasks"),
    )
    .unwrap();
    let store = open(dir.path());
    assert!(tine_store::publish::query_publication_destination(&store, "tasks").is_err());
    store.close();
}
