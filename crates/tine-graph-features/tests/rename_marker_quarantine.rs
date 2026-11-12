//! R-VCS-MARKERS (og 21a): marked pages stay byte-identical, including an
//! explicit source title. The report lists each skipped rewrite once.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages::{self, RenameOutcome};
use tine_store::{PageId, RenameMap, Store, TitleRebind};

fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

fn graph(label: &str, files: &[(&str, &str)]) -> (PathBuf, std::sync::Arc<Store>) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-rename-markers-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    (root, store)
}

const CONFLICTED: &str =
    "- intro\n<<<<<<< HEAD\n- mine sees [[Alpha]]\n=======\n- theirs sees [[Alpha]]\n>>>>>>> branch\n";

const MARKED_ORG: &str =
    "* [[Old]]\n*** skipped level\n<<<<<<< ours\n* x\n=======\n* y\n>>>>>>> theirs\n";

#[test]
fn single_rename_refuses_its_marked_org_source_that_does_not_round_trip() {
    let _integration_guard = crate::integration_lock();
    let source = format!("#+TITLE: Old\n{MARKED_ORG}");
    let (root, store) = graph("marked-org-source", &[("pages/Old.org", source.as_str())]);
    let refusal = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Old", "New", None, None, &[])
    })
    .unwrap_err();
    assert_eq!(refusal.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(refusal.to_string().contains("read-only .org file"));
    assert_eq!(
        fs::read_to_string(root.join("pages/Old.org")).unwrap(),
        source
    );
    assert!(!root.join("pages/New.org").exists());
}

#[test]
fn single_rename_refuses_a_held_marked_org_referrer_that_does_not_round_trip() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = graph(
        "marked-org-held",
        &[("pages/Old.org", "* body\n"), ("pages/r.org", MARKED_ORG)],
    );
    let refusal = hosted(&store, |host| {
        let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
            .as_u64()
            .unwrap();
        host.open(session, 1, &tine_store::PageId::from("pages/r.org"), "r")
            .unwrap();
        pages::rename_or_merge_page(&store, host, "Old", "New", None, None, &[])
    })
    .unwrap_err();
    assert_eq!(refusal.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(root.join("pages/Old.org").exists() && !root.join("pages/New.org").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/r.org")).unwrap(),
        MARKED_ORG
    );
}

#[test]
fn rename_skips_marker_bearing_referrers_and_reports_them() {
    let _integration_guard = crate::integration_lock();
    let source = "title:: Alpha\n\n- alpha\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n";
    let (root, store) = graph(
        "referrer",
        &[
            ("pages/Alpha.md", source),
            ("pages/Conflicted.md", CONFLICTED),
            ("pages/Clean.md", "- clean sees [[Alpha]]\n"),
        ],
    );
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Alpha", "Beta", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.outcome, RenameOutcome::Renamed);
    assert_eq!(
        fs::read_to_string(root.join("pages/Conflicted.md")).unwrap(),
        CONFLICTED
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Clean.md")).unwrap(),
        "- clean sees [[Beta]]\n"
    );
    assert!(root.join("pages/Beta.md").exists() && !root.join("pages/Alpha.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/Beta.md")).unwrap(),
        source
    );
    assert_eq!(
        report.skipped_conflicted_referrers,
        vec![
            "pages/Alpha.md".to_owned(),
            "pages/Conflicted.md".to_owned()
        ]
    );
    assert!(report
        .touched
        .iter()
        .all(|page| page.path != "pages/Conflicted.md"));
    let _ = fs::remove_dir_all(root);
}

/// A namespace cascade hits one quarantined referrer with several pairs: it
/// still comes out byte-identical and is reported once. A marker-bearing
/// descendant that moves keeps its bytes exactly.
#[test]
fn namespace_rename_also_skips_marker_bearing_referrers_and_moves_them_verbatim() {
    let _integration_guard = crate::integration_lock();
    let conflicted = "- intro\n<<<<<<< HEAD\n- [[Parent]] and [[Parent/Child]]\n=======\n- [[Parent/Child]] only\n>>>>>>> branch\n";
    let child = "- child of [[Parent]]\n<<<<<<< HEAD\n- a\n=======\n- b\n>>>>>>> x\n";
    let (root, store) = graph(
        "namespace",
        &[
            ("pages/Parent.md", "- parent\n"),
            ("pages/Parent___Child.md", child),
            ("pages/Conflicted.md", conflicted),
        ],
    );
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Parent", "Ancestor", None, None, &[])
    })
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("pages/Conflicted.md")).unwrap(),
        conflicted
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/Ancestor___Child.md")).unwrap(),
        child
    );
    assert!(!root.join("pages/Parent___Child.md").exists());
    let mut skipped = report.skipped_conflicted_referrers.clone();
    skipped.sort();
    assert_eq!(
        skipped,
        vec![
            "pages/Conflicted.md".to_owned(),
            "pages/Parent___Child.md".to_owned()
        ]
    );
    let _ = fs::remove_dir_all(root);
}

/// The store backstop: a reference rewrite that would change a marker-bearing
/// file is refused whoever asks; the bytes stay.
#[test]
fn the_store_refuses_a_reference_rewrite_inside_a_marker_bearing_file() {
    let _integration_guard = crate::integration_lock();
    let (root, store) = graph("backstop", &[("pages/Conflicted.md", CONFLICTED)]);
    let id = PageId::from("pages/Conflicted.md");
    let (_, rev) = store.read(&id.file(), None).unwrap();
    let mut tx = store.page_plan(Some(tine_store::EditKind::RenamePage));
    tx.rewrite_refs(
        &id,
        rev,
        None,
        &std::sync::Arc::new(RenameMap(vec![("alpha".into(), "Beta".into())], Vec::new())),
        TitleRebind::Keep,
    );
    assert!(tx.plan().is_err());
    assert_eq!(
        fs::read_to_string(root.join("pages/Conflicted.md")).unwrap(),
        CONFLICTED
    );
    let _ = fs::remove_dir_all(root);
}
