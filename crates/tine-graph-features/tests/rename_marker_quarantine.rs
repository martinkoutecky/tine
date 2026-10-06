//! og 21a (master a8fd4230d): a rename never rewrites references inside a file
//! carrying VCS conflict markers. Such a referrer stays byte-identical and
//! quarantined, every other referrer is rewritten, and the skip is reported
//! once per file. The store refuses such a rewrite from any other caller.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages::{self, RenameOutcome};
use tine_store::{PageId, RenameMap, Store, TitleRebind, TxOutcome};

fn graph(label: &str, files: &[(&str, &str)]) -> (PathBuf, Store) {
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
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

const CONFLICTED: &str =
    "- intro\n<<<<<<< HEAD\n- mine sees [[Alpha]]\n=======\n- theirs sees [[Alpha]]\n>>>>>>> branch\n";

#[test]
fn rename_skips_marker_bearing_referrers_and_reports_them() {
    let (root, store) = graph(
        "referrer",
        &[
            ("pages/Alpha.md", "- alpha\n"),
            ("pages/Conflicted.md", CONFLICTED),
            ("pages/Clean.md", "- clean sees [[Alpha]]\n"),
        ],
    );
    let report = pages::rename_or_merge_page(&store, "Alpha", "Beta", None, None, &[]).unwrap();
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
        report.skipped_conflicted_referrers,
        vec!["pages/Conflicted.md".to_owned()]
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
    let report =
        pages::rename_or_merge_page(&store, "Parent", "Ancestor", None, None, &[]).unwrap();
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
    let (root, store) = graph("backstop", &[("pages/Conflicted.md", CONFLICTED)]);
    let id = PageId::from("pages/Conflicted.md");
    let (_, rev) = store.read(&id.file(), None).unwrap();
    let mut tx = store.transaction(Some(tine_store::EditKind::RenamePage));
    tx.rewrite_refs(
        &id,
        rev,
        None,
        &RenameMap(vec![("alpha".into(), "Beta".into())]),
        TitleRebind::Keep,
    );
    assert!(matches!(tx.commit(), TxOutcome::NotCommitted { .. }));
    assert_eq!(
        fs::read_to_string(root.join("pages/Conflicted.md")).unwrap(),
        CONFLICTED
    );
    let _ = fs::remove_dir_all(root);
}
