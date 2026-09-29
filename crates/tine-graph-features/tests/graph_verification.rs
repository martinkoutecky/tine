//! Cross-device graph verifier (master 749bfb2b1): real nested sources, exact
//! bytes, no mutation, cancellation, and every race that must read incomplete.

use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use tine_graph_features::graph_verification::{verify_graph_bytes, Cancelled, Manifest};
use tine_store::{OpenOptions, Store};

fn graph(name: &str) -> (PathBuf, Store) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "tine-verify-{name}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("pages/nested")).unwrap();
    fs::create_dir_all(root.join("archive/deep")).unwrap();
    fs::write(
        root.join("pages/nested/Unicode 題.md"),
        b"- exact\r\nbytes\n",
    )
    .unwrap();
    fs::write(root.join("archive/deep/Elsewhere.org"), b"* elsewhere\n").unwrap();
    fs::write(root.join("pages/.hidden.md"), b"- private\n").unwrap();
    fs::write(root.join("archive/deep/not-graph.txt"), b"ignored\n").unwrap();
    let (store, _, _) = Store::open(&root, OpenOptions::default()).unwrap();
    (root, store)
}

fn run(store: &Store) -> Manifest {
    verify_graph_bytes(store, &|| false, &mut |_, _| {}).unwrap()
}

#[test]
fn real_nested_sources_are_hashed_exactly_without_mutation() {
    let (root, store) = graph("exact");
    let before = fs::read(root.join("pages/nested/Unicode 題.md")).unwrap();
    let manifest = run(&store);
    let paths: Vec<_> = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["archive/deep/Elsewhere.org", "pages/nested/Unicode 題.md"]
    );
    let unicode = &manifest.files[1];
    assert_eq!(unicode.length, before.len() as u64);
    assert_eq!(unicode.digest, format!("{:x}", Sha256::digest(&before)));
    assert!(manifest.complete && manifest.errors.is_empty());
    assert_eq!(manifest.total_bytes(), before.len() as u64 + 12);
    assert_eq!(
        fs::read(root.join("pages/nested/Unicode 題.md")).unwrap(),
        before
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_report_round_trips_and_a_foreign_format_is_refused() {
    let (root, store) = graph("report");
    let manifest = run(&store);
    let text = manifest.to_report().unwrap();
    assert!(text.contains("\"schemaVersion\": 1") && text.contains("\"aggregateDigest\""));
    assert_eq!(Manifest::from_report(&text).unwrap().files, manifest.files);
    assert!(Manifest::from_report(&text.replace("tine-graph-bytes", "other")).is_err());
    assert!(
        Manifest::from_report(&text.replace("\"schemaVersion\": 1", "\"schemaVersion\": 2"))
            .is_err()
    );
    assert!(Manifest::from_report("not json").is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_same_size_byte_change_moves_the_aggregate_and_the_file_digest() {
    let (root, store) = graph("aggregate");
    let first = run(&store);
    fs::write(root.join("archive/deep/Elsewhere.org"), b"* elsewherE\n").unwrap();
    let second = run(&store);
    assert_eq!(first.files[0].length, second.files[0].length);
    assert_ne!(first.files[0].digest, second.files[0].digest);
    assert_ne!(first.aggregate_digest, second.aggregate_digest);
    assert_eq!(run(&store).aggregate_digest, second.aggregate_digest);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn cancellation_stops_the_run_and_leaves_the_bytes_alone() {
    let (root, store) = graph("cancel");
    let before = fs::read(root.join("archive/deep/Elsewhere.org")).unwrap();
    let result = verify_graph_bytes(&store, &|| true, &mut |_, _| {});
    assert_eq!(result.unwrap_err(), Cancelled);
    let polls = Cell::new(0);
    let late = verify_graph_bytes(
        &store,
        &|| {
            polls.set(polls.get() + 1);
            polls.get() > 3
        },
        &mut |_, _| {},
    );
    assert_eq!(late.unwrap_err(), Cancelled);
    assert_eq!(
        fs::read(root.join("archive/deep/Elsewhere.org")).unwrap(),
        before
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_file_set_that_changes_mid_run_is_incomplete_never_a_match() {
    let (root, store) = graph("race-add");
    let manifest = verify_graph_bytes(&store, &|| false, &mut |done, _| {
        if done == 1 {
            fs::write(root.join("pages/Arrived.md"), b"- new\n").unwrap();
        }
    })
    .unwrap();
    assert!(!manifest.complete);
    assert!(manifest.aggregate_digest.is_none());
    assert!(manifest
        .errors
        .iter()
        .any(|e| e.detail.contains("changed while verification")));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_file_removed_after_listing_is_reported_by_path() {
    let (root, store) = graph("race-remove");
    let manifest = verify_graph_bytes(&store, &|| false, &mut |done, total| {
        if done == 0 && total > 0 {
            fs::remove_file(root.join("archive/deep/Elsewhere.org")).unwrap();
        }
    })
    .unwrap();
    assert!(!manifest.complete);
    assert!(manifest
        .errors
        .iter()
        .any(|e| e.path.as_deref() == Some("archive/deep/Elsewhere.org")));
    let _ = fs::remove_dir_all(&root);
}
