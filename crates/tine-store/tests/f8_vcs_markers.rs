#![cfg(feature = "test-faults")]
//! og family 8b (Concord): refusal R-VCS-MARKERS and the one exempt write,
//! `SaveBase::ResolvingMarkers`. Threat scenario for the refusal: a VCS
//! merge by an external writer left unresolved markers; a rewrite would
//! re-indent them and silently lose a side. Ported from master
//! `marker_bearing_page_is_never_rewritten_by_save` and
//! `resolving_markers_stages_the_preresolution_file_in_recoverable_trash`.
//! The staged copy's crash and failure proofs run the real resolution in
//! `tine-graph-features/tests/host_op_conflicts.rs`.

use crate::test_host_writer::{HostWriter, SaveRefused};
use crate::{EditKind, OpenOptions, PageId, Refusal, SaveBase, Store, Why};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tine_core::model::{BlockDto, Format, PageDto, PageKind};

const MARKED: &str =
    "<<<<<<< HEAD\n- mine\n||||||| base\n- old\n=======\n- theirs\n>>>>>>> feature\n";

fn scratch(label: &str) -> PathBuf {
    let root = crate::test_support::fresh_dir(&format!("tine-f8-{label}-"));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn open(root: &Path) -> Arc<Store> {
    Arc::new(Store::open(root, OpenOptions::default()).unwrap().0)
}

fn page(raw: &str) -> PageDto {
    PageDto {
        name: "Merge".into(),
        title: "Merge".into(),
        kind: PageKind::Page,
        format: Format::Md,
        blocks: vec![BlockDto {
            id: "resolved".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        pre_block: None,
        rev: None,
        read_only: false,
        guide: false,
    }
}

fn staged_copies(root: &Path) -> Vec<Vec<u8>> {
    let Ok(entries) = fs::read_dir(root.join("logseq/.tine-trash/conflicts")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with("__markers__Merge.md")
        })
        .map(|e| fs::read(e.path()).unwrap())
        .collect()
}

#[test]
fn marker_bearing_page_is_never_rewritten_by_save() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let root = scratch("refuse");
    let file = root.join("pages/Merge.md");
    fs::write(&file, MARKED).unwrap();
    let store = open(&root);
    let id = PageId::from("pages/Merge.md");
    let read = store
        .page(&id)
        .expect("a marker-bearing page stays readable");
    assert!(!read.doc.blocks.is_empty());
    let mut edited = read.doc.clone();
    edited.blocks[0].raw = "mine edited".into();
    let outcome = HostWriter::for_store(&store).save(
        EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &edited,
    );
    let Err(SaveRefused::Planned(Why::Refused(Refusal::ReadOnly(message)))) = outcome else {
        panic!("R-VCS-MARKERS: a save to a marker page must refuse, got {outcome:?}");
    };
    assert!(
        message.contains("<<<<<<<") && message.contains(">>>>>>>"),
        "the refusal names the markers it found: {message}"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        MARKED,
        "bytes untouched"
    );
    assert!(staged_copies(&root).is_empty(), "a refusal stages nothing");
    // A page that merely documents git (fenced) saves normally.
    let doc_file = root.join("pages/Docs.md");
    fs::write(&doc_file, "- about git\n```\n<<<<<<< HEAD\n```\n").unwrap();
    let store = open(&root);
    let docs = PageId::from("pages/Docs.md");
    let read = store.page(&docs).unwrap();
    let mut edited = read.doc.clone();
    edited.blocks[0].raw = "about git, edited".into();
    HostWriter::for_store(&store)
        .save(
            EditKind::ReplacePage,
            &docs,
            SaveBase::Existing(read.rev),
            &edited,
        )
        .unwrap();
    let _ = fs::remove_dir_all(&root);
}

/// The exemption lives only in its operation: the resolved page is ordinary
/// and an ordinary save to another marker page is refused again.
#[test]
fn a_resolved_page_saves_and_another_marker_page_still_refuses() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let root = scratch("resolve");
    let file = root.join("pages/Merge.md");
    fs::write(&file, MARKED).unwrap();
    fs::write(root.join("pages/Other.md"), MARKED).unwrap();
    let store = open(&root);
    let writer = HostWriter::for_store(&store);
    let id = PageId::from("pages/Merge.md");
    let rev = store.read(&id.file(), None).unwrap().1;
    let mut tx = store.page_plan(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::ResolvingMarkers(rev),
        &page("mine"),
    );
    writer.apply(tx.plan().unwrap()).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "- mine\n");
    let other = PageId::from("pages/Other.md");
    let read = store.page(&other).unwrap();
    assert!(matches!(
        writer.save(
            EditKind::ReplacePage,
            &other,
            SaveBase::Existing(read.rev),
            &read.doc
        ),
        Err(SaveRefused::Planned(Why::Refused(Refusal::ReadOnly(_))))
    ));
    let read = store.page(&id).unwrap();
    let mut edited = read.doc.clone();
    edited.blocks[0].raw = "mine again".into();
    writer
        .save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &edited,
        )
        .unwrap();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_stale_resolution_writes_and_stages_nothing() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let root = scratch("stale");
    let file = root.join("pages/Merge.md");
    fs::write(&file, MARKED).unwrap();
    let store = open(&root);
    let id = PageId::from("pages/Merge.md");
    let rev = store.read(&id.file(), None).unwrap().1;
    // The VCS rewrote the file after the review was computed.
    let newer = format!("{MARKED}- appended by the merge\n");
    fs::write(&file, &newer).unwrap();
    let mut tx = store.page_plan(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::ResolvingMarkers(rev),
        &page("mine"),
    );
    let outcome = tx.plan();
    assert!(matches!(outcome, Err(Why::Conflict { .. })), "{outcome:?}");
    assert_eq!(fs::read_to_string(&file).unwrap(), newer);
    assert!(
        !root.join("logseq/.tine-trash").exists(),
        "a refused resolve must not materialize a trash directory"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_marker_page_refusal_names_its_refusal_family() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let root = scratch("family");
    fs::write(root.join("pages/Merge.md"), MARKED).unwrap();
    let store = open(&root);
    let id = PageId::from("pages/Merge.md");
    let read = store.page(&id).unwrap();
    let mut tx = store.page_plan(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::Existing(read.rev),
        &read.doc,
    );
    let outcome = tx.plan();
    assert!(
        matches!(outcome, Err(Why::Refused(Refusal::ReadOnly(_)))),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&root);
}
