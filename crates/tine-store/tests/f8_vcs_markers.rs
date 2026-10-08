#![cfg(feature = "test-faults")]
//! og family 8b (Concord): refusal R-VCS-MARKERS and the one exempt write,
//! `SaveBase::ResolvingMarkers`. Threat scenario for the refusal: a VCS
//! merge by an external writer left unresolved markers; a rewrite would
//! re-indent them and silently lose a side. Ported from master
//! `marker_bearing_page_is_never_rewritten_by_save` and
//! `resolving_markers_stages_the_preresolution_file_in_recoverable_trash`,
//! plus og's crash and rollback proofs for the staged copy.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{
    EditKind, FaultPoint, OpenOptions, PageId, Refusal, SaveBase, SaveOutcome, Store, TxOutcome,
    Why,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

const MARKED: &str =
    "<<<<<<< HEAD\n- mine\n||||||| base\n- old\n=======\n- theirs\n>>>>>>> feature\n";

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "tine-f8-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn open(root: &Path) -> Store {
    Store::open(root, OpenOptions::default()).unwrap().0
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
    let outcome = store.save(
        EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &edited,
    );
    let SaveOutcome::ReadOnly(message) = outcome else {
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
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &docs,
            SaveBase::Existing(read.rev),
            &edited
        ),
        SaveOutcome::Saved(_)
    ));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolving_markers_stages_the_preresolution_file_in_recoverable_trash() {
    let root = scratch("resolve");
    let file = root.join("pages/Merge.md");
    fs::write(&file, MARKED).unwrap();
    let store = open(&root);
    let id = PageId::from("pages/Merge.md");
    let rev = store.read(&id.file(), None).unwrap().1;
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::ResolvingMarkers(rev),
        &page("mine"),
    );
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    assert_eq!(fs::read_to_string(&file).unwrap(), "- mine\n");
    assert_eq!(
        staged_copies(&root),
        vec![MARKED.as_bytes().to_vec()],
        "exactly one byte-exact recovery copy of the pre-resolution file"
    );
    // The exemption lived only in that transaction: the page is ordinary now,
    // and an ordinary save to another marker page is refused again.
    fs::write(root.join("pages/Other.md"), MARKED).unwrap();
    let store = open(&root);
    let other = PageId::from("pages/Other.md");
    let read = store.page(&other).unwrap();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &other,
            SaveBase::Existing(read.rev),
            &read.doc
        ),
        SaveOutcome::ReadOnly(_)
    ));
    let read = store.page(&id).unwrap();
    let mut edited = read.doc.clone();
    edited.blocks[0].raw = "mine again".into();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &edited
        ),
        SaveOutcome::Saved(_)
    ));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_stale_resolution_writes_and_stages_nothing() {
    let root = scratch("stale");
    let file = root.join("pages/Merge.md");
    fs::write(&file, MARKED).unwrap();
    let store = open(&root);
    let id = PageId::from("pages/Merge.md");
    let rev = store.read(&id.file(), None).unwrap().1;
    // The VCS rewrote the file after the review was computed.
    let newer = format!("{MARKED}- appended by the merge\n");
    fs::write(&file, &newer).unwrap();
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::ResolvingMarkers(rev),
        &page("mine"),
    );
    let outcome = tx.commit();
    assert!(
        matches!(
            outcome,
            TxOutcome::NotCommitted {
                why: Why::Conflict { .. },
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), newer);
    assert!(
        !root.join("logseq/.tine-trash").exists(),
        "a refused resolve must not materialize a trash directory"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_failed_resolution_restores_the_file_and_withdraws_its_copy() {
    let root = scratch("rollback");
    let file = root.join("pages/Merge.md");
    fs::write(&file, MARKED).unwrap();
    fs::write(root.join("pages/Plain.md"), "- plain\n").unwrap();
    let store = open(&root);
    let id = PageId::from("pages/Merge.md");
    let plain = PageId::from("pages/Plain.md");
    let rev = store.read(&id.file(), None).unwrap().1;
    let plain_rev = store.read(&plain.file(), None).unwrap().1;
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::ResolvingMarkers(rev),
        &page("mine"),
    );
    let mut plain_doc = page("plain edited");
    plain_doc.name = "Plain".into();
    plain_doc.title = "Plain".into();
    tx.save_page(
        &[EditKind::ReplacePage],
        &plain,
        SaveBase::Existing(plain_rev),
        &plain_doc,
    );
    store.inject_fault(FaultPoint::MidStepIoAt(1));
    let outcome = tx.commit();
    assert!(
        matches!(
            outcome,
            TxOutcome::NotCommitted {
                why: Why::Failed(_),
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        MARKED,
        "undo restored the markers"
    );
    assert!(
        staged_copies(&root).is_empty(),
        "the copy is withdrawn once the original is back"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_marker_page_refusal_names_its_refusal_family() {
    let root = scratch("family");
    fs::write(root.join("pages/Merge.md"), MARKED).unwrap();
    let store = open(&root);
    let id = PageId::from("pages/Merge.md");
    let read = store.page(&id).unwrap();
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::Existing(read.rev),
        &read.doc,
    );
    let outcome = tx.commit();
    assert!(
        matches!(
            outcome,
            TxOutcome::NotCommitted {
                why: Why::Refused(Refusal::ReadOnly(_)),
                ..
            }
        ),
        "{outcome:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn crash_marker_resolution_worker() {
    let Ok(root) = std::env::var("TINE_F8_CRASH_ROOT") else {
        return;
    };
    let boundary = std::env::var("TINE_F8_CRASH_BOUNDARY").unwrap();
    let root = Path::new(&root);
    let store = open(root);
    let id = PageId::from("pages/Merge.md");
    let rev = store.read(&id.file(), None).unwrap().1;
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::ResolvingMarkers(rev),
        &page("mine"),
    );
    store.inject_fault(if boundary == "stage" {
        FaultPoint::AbortAfterMarkerStage
    } else {
        FaultPoint::AbortAfterStep(0)
    });
    let _ = tx.commit();
    panic!("the marker resolution fault did not abort");
}

/// Kill the process after the pre-resolution copy is durable (before the
/// replacement) and after the replacement: on reopen the page is the old or
/// the new state and the pre-resolution bytes survive in trash either way.
#[test]
fn a_crash_during_resolution_never_loses_a_side() {
    for (boundary, expected_live) in [("stage", MARKED), ("written", "- mine\n")] {
        let root = scratch("crash");
        fs::write(root.join("pages/Merge.md"), MARKED).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("crash_marker_resolution_worker")
            .arg("--nocapture")
            .env("TINE_F8_CRASH_ROOT", &root)
            .env("TINE_F8_CRASH_BOUNDARY", boundary)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "child must abort at {boundary}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let store = open(&root);
        let (live, _) = store
            .read(&PageId::from("pages/Merge.md").file(), None)
            .unwrap();
        assert_eq!(
            String::from_utf8(live).unwrap(),
            expected_live,
            "{boundary}"
        );
        assert_eq!(
            staged_copies(&root),
            vec![MARKED.as_bytes().to_vec()],
            "{boundary}: the pre-resolution bytes are recoverable"
        );
        store.close();
        let _ = fs::remove_dir_all(&root);
    }
}
