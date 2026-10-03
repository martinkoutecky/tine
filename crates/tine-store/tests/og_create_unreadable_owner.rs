//! Port of master 69e0a885ddf9 + 69525c055f0b (GH #543): creating a page by
//! name refuses, naming the file, when a page file Tine cannot read could be
//! that page. Threat scenario `DIRECT-REF-CREATE-UNREADABLE-OWNER`
//! (docs/storage-contract.md): sync delivery, an interrupted external write or
//! malformed imported Markdown/Org leaves a file whose effective name is
//! unknown; a second file for that name would give one name two owners once
//! the bad file is repaired.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{OpenOptions, PageId, Resolved, SaveBase, SaveOutcome, Store, WatchMode};

fn graph(files: &[(&str, &[u8])]) -> (PathBuf, Store) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-og-create-unreadable-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, bytes) in files {
        fs::write(root.join(rel), bytes).unwrap();
    }
    let store = Store::open(
        &root,
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    store.whole_graph().unwrap();
    (root, store)
}

fn doc(name: &str) -> PageDto {
    PageDto {
        name: name.into(),
        kind: PageKind::Page,
        title: name.into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b".into(),
            raw: "new page".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        guide: false,
    }
}

/// The user's path: resolve the name, then save the proposed id as CreateNew.
fn create_by_name(store: &Store, name: &str) -> (PageId, SaveOutcome) {
    let id = match store.whole_graph().unwrap().resolve(name, false) {
        Resolved::Absent { id } => id,
        _ => panic!("{name} should resolve as absent"),
    };
    let outcome = store.save(tine_store::EditKind::CreatePage, &id, SaveBase::CreateNew, &doc(name));
    (id, outcome)
}

// A Latin-1 property line inside the preamble makes the file undecodable
// before its name is settled, so its effective name is unknown; its bytes still
// say `title:: Target`.
const UNREADABLE_TARGET: &[u8] = b"title:: Target\ntags:: caf\xe9\n\n- body\n";

#[test]
fn create_refuses_a_name_an_unreadable_file_could_own_and_names_the_file() {
    let (root, store) = graph(&[("pages/Other.md", UNREADABLE_TARGET)]);
    assert!(
        store.whole_graph().unwrap().unreadable_files().iter().any(|(id, _)| id.as_str() == "pages/Other.md"),
        "precondition: Other.md is unreadable"
    );
    let (id, outcome) = create_by_name(&store, "Target");
    match &outcome {
        SaveOutcome::UnreadableOwner { file } => assert_eq!(
            file.as_str(),
            "pages/Other.md",
            "the refusal names the unreadable file"
        ),
        other => panic!("creating Target must be refused, got {other:?}"),
    }
    assert!(
        !root.join(id.file().as_str()).exists(),
        "no second file claims Target"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_unreadable_file_blocks_only_names_it_could_be() {
    let (root, store) = graph(&[("pages/Other.md", UNREADABLE_TARGET)]);
    let (id, outcome) = create_by_name(&store, "Unrelated");
    assert!(
        matches!(outcome, SaveOutcome::Saved(_)),
        "a name the unreadable file cannot be is created: {outcome:?}"
    );
    assert!(root.join(id.file().as_str()).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_repaired_file_no_longer_blocks_its_name() {
    let (root, store) = graph(&[("pages/Other.md", UNREADABLE_TARGET)]);
    fs::write(root.join("pages/Other.md"), b"title:: Target\ntags:: cafe\n\n- body\n").unwrap();
    store.scan_refresh().unwrap();
    // Now readable: Target exists and resolves to Other.md.
    assert!(matches!(
        store.whole_graph().unwrap().resolve("Target", false),
        Resolved::Existing { .. }
    ));
    let (_, outcome) = create_by_name(&store, "Fresh");
    assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_parser_rejected_file_with_a_readable_title_blocks_only_that_title() {
    // Too deep for the parser, but the preamble still names the page, so a
    // mere mention of another name in its body blocks nothing.
    let mut bytes = b"title:: Known\n\n".to_vec();
    for depth in 0..600 {
        bytes.extend(" ".repeat(depth * 2).as_bytes());
        bytes.extend(b"- mentions [[Target]]\n");
    }
    let (root, store) = graph(&[("pages/Deep.md", &bytes)]);
    assert!(
        store.whole_graph().unwrap().unreadable_files().iter().any(|(id, _)| id.as_str() == "pages/Deep.md"),
        "precondition: the parser rejects Deep.md"
    );
    let (_, outcome) = create_by_name(&store, "Target");
    assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn create_unique_of_a_page_is_refused_too() {
    let (root, store) = graph(&[("pages/Other.md", UNREADABLE_TARGET)]);
    let mut tx = store.transaction(Some(tine_store::EditKind::CreatePage));
    tx.create_unique(
        tine_store::Area::Pages,
        "Target",
        ".md",
        tine_store::Content::Bytes(b"- new\n".to_vec()),
    );
    match tx.commit() {
        tine_store::TxOutcome::NotCommitted {
            why: tine_store::Why::Refused(tine_store::Refusal::UnreadableOwner { file }),
            ..
        } => assert_eq!(file.as_str(), "pages/Other.md"),
        other => panic!("expected the unreadable-owner refusal, got {other:?}"),
    }
    assert!(!root.join("pages/Target.md").exists());
    let _ = fs::remove_dir_all(root);
}

// The listing skips a FIFO today; `unreadable_page_could_own` also answers
// "no page" for one (unit test in model/page_identity.rs), so neither layer can
// turn a stray pipe into a blanket creation refusal (master audit R15-05).
#[cfg(unix)]
#[test]
fn a_fifo_in_pages_blocks_no_creation() {
    let (root, store) = graph(&[]);
    let fifo = root.join("pages/Target-pipe.md");
    let status = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success());
    store.scan_refresh().unwrap();
    let (_, outcome) = create_by_name(&store, "Target");
    assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
    let _ = fs::remove_dir_all(root);
}
