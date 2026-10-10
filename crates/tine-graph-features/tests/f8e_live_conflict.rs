//! og family 8e (Concord live drafts): an editor draft whose guarded save was
//! refused because the file changed on disk is reviewed against the disk as it
//! is now and resolved through one guarded store transaction. Ported from
//! master `concord_live_save_conflict_*` / `durable_draft_*` tests to og's
//! Store-based feature client.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::PageDto;
use tine_core::sync_diff::{DiffRow, RowKind};
use tine_graph_features::live_conflict::{
    live_conflict_diff, merge_live_conflict, resolve_live_conflict, ABSENT,
};
use tine_store::{FileRev, PageId, Store};

const PAGE: &str = "pages/Desk.md";
const BASE: &str = "- the shared intro line\n- the plan for today\n";

fn scratch(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-f8e-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn open(root: &Path) -> Store {
    Store::open(root, Default::default()).unwrap().0
}

/// The editor's draft: the page as loaded, with block `index` retyped.
fn draft(store: &Store, index: usize, text: &str) -> (PageDto, String) {
    let read = store.page(&PageId::from(PAGE)).unwrap();
    let mut page = read.doc;
    page.blocks[index].raw = text.to_owned();
    let rev: String = read.rev.into();
    (page, rev)
}

fn external(root: &Path, text: &str) {
    let path = root.join(PAGE);
    let temp = path.with_extension("ext-tmp");
    fs::write(&temp, text).unwrap();
    fs::rename(temp, path).unwrap();
}

fn preselected(rows: &[DiffRow], out: &mut HashMap<String, String>) {
    for row in rows {
        let choice = row.suggestion.clone().unwrap_or_else(|| {
            if row.kind == RowKind::Unchanged {
                "mine"
            } else {
                "both"
            }
            .to_owned()
        });
        out.insert(row.id.clone(), choice);
        preselected(&row.children, out);
    }
}

fn decisions(diff: &tine_core::sync_diff::SyncConflictDiff) -> HashMap<String, String> {
    let mut out = HashMap::new();
    preselected(&diff.rows, &mut out);
    out
}

/// The editor loaded BASE and retyped one block while another editor retyped a
/// different one. With the ledger's copy of BASE the review is 3-way, each
/// side's edit arrives pre-selected, and Apply writes both edits through one
/// guarded save at the reviewed disk revision.
#[test]
fn a_live_conflict_uses_the_editor_base_and_guarded_resolution() {
    let root = scratch("three-way");
    fs::write(root.join(PAGE), BASE).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, "the plan for tomorrow");
    external(&root, "- the shared intro lines\n- the plan for today\n");
    let bases = vec!["- unrelated\n".to_owned(), BASE.to_owned()];
    let diff = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &bases).unwrap();
    assert!(diff.three_way);
    assert!(diff.merge_base_rev.is_some());
    assert_eq!(diff.base_rev, base_rev);
    let disk_rev: String =
        FileRev::from_bytes(fs::read(root.join(PAGE)).unwrap().as_slice()).into();
    assert_eq!(diff.conflict_rev, disk_rev);
    let chosen = decisions(&diff);
    let resolved = resolve_live_conflict(
        &store,
        None,
        PAGE,
        &page,
        &diff.conflict_rev,
        diff.merge_base_rev.as_deref(),
        &bases,
        &chosen,
        "union",
    )
    .unwrap();
    let written = fs::read_to_string(root.join(PAGE)).unwrap();
    assert_eq!(
        written,
        "- the shared intro lines\n- the plan for tomorrow\n"
    );
    let written_rev: String = FileRev::from_bytes(written.as_bytes()).into();
    assert_eq!(resolved.rev.as_deref(), Some(written_rev.as_str()));
    let _ = fs::remove_dir_all(root);
}

/// No retained text has the draft's revision: the review is 2-way, nothing is
/// pre-selected (keep both), and a forged `"merged"` decision refuses.
#[test]
fn without_the_editor_base_the_review_is_two_way_and_keeps_both() {
    let root = scratch("two-way");
    fs::write(root.join(PAGE), BASE).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, "the plan, revised here");
    external(&root, "- shared intro\n- the plan, revised there\n");
    let foreign = vec!["- shared intro\n- the plan, another text\n".to_owned()];
    let diff = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &foreign).unwrap();
    assert!(!diff.three_way && diff.merge_base_rev.is_none());
    let chosen = decisions(&diff);
    let mut forged = chosen.clone();
    for value in forged.values_mut().filter(|v| *v == "both") {
        *value = "merged".into();
    }
    assert!(resolve_live_conflict(
        &store,
        None,
        PAGE,
        &page,
        &diff.conflict_rev,
        None,
        &foreign,
        &forged,
        "union",
    )
    .is_err());
    resolve_live_conflict(
        &store,
        None,
        PAGE,
        &page,
        &diff.conflict_rev,
        None,
        &foreign,
        &chosen,
        "union",
    )
    .unwrap();
    let written = fs::read_to_string(root.join(PAGE)).unwrap();
    for text in [
        "the plan, revised here",
        "the plan, revised there",
        "shared intro",
    ] {
        assert!(written.contains(text), "{text:?} lost: {written}");
    }
    let _ = fs::remove_dir_all(root);
}

/// A write after the review refuses the apply and writes nothing; a fresh
/// review of the newer disk then resolves.
#[test]
fn a_newer_external_write_refuses_and_writes_nothing_until_rereviewed() {
    let root = scratch("newer");
    fs::write(root.join(PAGE), BASE).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, "the plan, revised here");
    external(&root, "- shared intro\n- first outside edit\n");
    let diff = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &[]).unwrap();
    external(&root, "- shared intro\n- second outside edit\n");
    let err = resolve_live_conflict(
        &store,
        None,
        PAGE,
        &page,
        &diff.conflict_rev,
        None,
        &[],
        &decisions(&diff),
        "union",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        fs::read_to_string(root.join(PAGE)).unwrap(),
        "- shared intro\n- second outside edit\n"
    );
    let fresh = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &[]).unwrap();
    assert_ne!(fresh.conflict_rev, diff.conflict_rev);
    resolve_live_conflict(
        &store,
        None,
        PAGE,
        &page,
        &fresh.conflict_rev,
        None,
        &[],
        &decisions(&fresh),
        "union",
    )
    .unwrap();
    let written = fs::read_to_string(root.join(PAGE)).unwrap();
    assert!(
        written.contains("second outside edit") && written.contains("the plan, revised here"),
        "{written}"
    );
    let _ = fs::remove_dir_all(root);
}

/// The ledger moved between review and apply (sync delivery or an honest
/// concurrent instance): a `"merged"` row refuses rather than composing a body
/// against a base the user did not see; the draft and disk are untouched.
#[test]
fn a_stale_ledger_base_refuses_a_merged_row_and_loses_nothing() {
    let root = scratch("stale-base");
    const ID: &str = "aaaaaaaa-0000-0000-0000-0000000000e8";
    let body = |text: &str| format!("- shared intro\n- {text}\n  id:: {ID}\n");
    let base = body("Desktop 5");
    fs::write(root.join(PAGE), &base).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, &format!("Desktop\nid:: {ID}"));
    external(&root, &body("Desktop 5 kk"));
    let bases = vec![base.to_owned()];
    let diff = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &bases).unwrap();
    let chosen = decisions(&diff);
    assert!(chosen.values().any(|d| d == "merged"), "{:?}", diff.rows);
    for moved in [vec![], vec!["- shared intro\n- other\n".to_owned()]] {
        let err = resolve_live_conflict(
            &store,
            None,
            PAGE,
            &page,
            &diff.conflict_rev,
            diff.merge_base_rev.as_deref(),
            &moved,
            &chosen,
            "union",
        )
        .unwrap_err();
        assert!(err.to_string().contains("merge base changed"), "{err}");
    }
    assert_eq!(
        fs::read_to_string(root.join(PAGE)).unwrap(),
        body("Desktop 5 kk")
    );
    assert!(page.blocks[1].raw.starts_with("Desktop\n"));
    resolve_live_conflict(
        &store,
        None,
        PAGE,
        &page,
        &diff.conflict_rev,
        diff.merge_base_rev.as_deref(),
        &bases,
        &chosen,
        "union",
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(PAGE)).unwrap(),
        body("Desktop kk")
    );
    let _ = fs::remove_dir_all(root);
}

/// Step 3b R6: the merge the page host's client submits is read-only. A
/// `"merged"` row reads the retained text the review named, wherever the
/// ledger now lists it among newer texts; the disk is untouched.
#[test]
fn a_merge_reads_the_reviewed_base_by_its_name_and_writes_nothing() {
    let root = scratch("merge-read-only");
    const ID: &str = "aaaaaaaa-0000-0000-0000-0000000000e9";
    let body = |text: &str| format!("- shared intro\n- {text}\n  id:: {ID}\n");
    let base = body("Desktop 5");
    fs::write(root.join(PAGE), &base).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, &format!("Desktop\nid:: {ID}"));
    external(&root, &body("Desktop 5 kk"));
    let diff = live_conflict_diff(
        &store,
        PAGE,
        &page,
        Some(&base_rev),
        std::slice::from_ref(&base),
    )
    .unwrap();
    let chosen = decisions(&diff);
    assert!(chosen.values().any(|d| d == "merged"), "{:?}", diff.rows);
    let grown = vec![body("Desktop 6"), base];
    let merged = merge_live_conflict(
        &store,
        PAGE,
        &page,
        &diff.conflict_rev,
        diff.merge_base_rev.as_deref(),
        &grown,
        &chosen,
        "union",
    )
    .unwrap();
    assert_eq!(merged.blocks[1].raw, format!("Desktop kk\nid:: {ID}"));
    assert_eq!(merged.rev, None, "a merge has no written revision");
    assert_eq!(
        fs::read_to_string(root.join(PAGE)).unwrap(),
        body("Desktop 5 kk"),
        "the merge wrote"
    );
    let _ = fs::remove_dir_all(root);
}

/// A deleted file reviews as `absent` without recreating it; Apply creates it,
/// and a file that reappeared meanwhile (even empty) refuses the apply.
#[test]
fn an_absent_file_review_is_read_only_and_apply_recreates_only_if_still_absent() {
    let root = scratch("absent");
    fs::write(root.join(PAGE), BASE).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, "retained draft");
    fs::remove_file(root.join(PAGE)).unwrap();
    let diff = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &[]).unwrap();
    assert_eq!(diff.conflict_rev, ABSENT);
    assert!(
        !root.join(PAGE).exists(),
        "the review must not recreate the file"
    );
    fs::write(root.join(PAGE), "").unwrap();
    let err = resolve_live_conflict(
        &store,
        None,
        PAGE,
        &page,
        ABSENT,
        None,
        &[],
        &decisions(&diff),
        "mine",
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(root.join(PAGE)).unwrap(), "");
    fs::remove_file(root.join(PAGE)).unwrap();
    let resolved = resolve_live_conflict(
        &store,
        None,
        PAGE,
        &page,
        ABSENT,
        None,
        &[],
        &decisions(&diff),
        "mine",
    )
    .unwrap();
    assert!(fs::read_to_string(root.join(PAGE))
        .unwrap()
        .contains("retained draft"));
    assert!(resolved.rev.is_some());
    let _ = fs::remove_dir_all(root);
}
