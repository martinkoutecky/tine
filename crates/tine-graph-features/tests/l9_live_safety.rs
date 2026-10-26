//! Audit L9-03 / L9-04 (2026-10-22 arrival audit) fail-before repros for the
//! Concord live-draft resolve (`live_conflict::merge_live_conflict`).
//! The window submits the merged page through its page host with the reviewed
//! disk revision as base (client.ts `submitReviewed`); this file uses the
//! store's guarded save with the same base, as f8e_live_conflict.rs does.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use std::sync::{mpsc, Arc};
use std::time::Duration;
use tine_core::model::PageDto;
use tine_core::sync_diff::{DiffRow, RowKind};
use tine_graph_features::live_conflict::{live_conflict_diff, merge_live_conflict};
use tine_store::{DiskToken, EditKind, FileRev, PageHost, PageId, PageOperation, Store};

const PAGE: &str = "pages/Desk.md";
const BASE: &str = "- the shared intro line\n- the plan for today\n";
const THEIRS_ONLY: &str = "an external editor's new block";

fn scratch(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-l9-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn open(root: &Path) -> Arc<Store> {
    Arc::new(Store::open(root, Default::default()).unwrap().0)
}

fn draft(store: &Store, index: usize, text: &str) -> (PageDto, String) {
    let read = store.page(&PageId::from(PAGE)).unwrap();
    let mut page = read.doc;
    page.blocks[index].raw = text.to_owned();
    (page, read.rev.into())
}

fn external(root: &Path, text: &str) {
    let path = root.join(PAGE);
    let temp = path.with_extension("ext-tmp");
    fs::write(&temp, text).unwrap();
    fs::rename(temp, path).unwrap();
}

/// The UI's opening position (DiffRows.tsx `seedSuggestedOrNoLoss`).
fn seeded(rows: &[DiffRow], out: &mut HashMap<String, String>) {
    for row in rows {
        if row.kind != RowKind::Unchanged {
            let no_loss = match row.kind {
                RowKind::Added => "mine",
                RowKind::Removed => "theirs",
                _ => "both",
            };
            out.insert(
                row.id.clone(),
                row.suggestion.clone().unwrap_or_else(|| no_loss.to_owned()),
            );
        }
        seeded(&row.children, out);
    }
}

fn save_guarded(store: &Arc<Store>, merged: &PageDto, conflict_rev: &str) {
    let app = tempfile::tempdir().unwrap();
    let (tx, rx) = mpsc::channel();
    let host = PageHost::start(
        store,
        app.path(),
        "l9",
        |_, _| {},
        move |mail| {
            tx.send(mail).unwrap();
        },
    )
    .unwrap();
    let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    let opened = host.open(session, 1, &PageId::from(PAGE), "Desk").unwrap();
    let key = serde_json::to_value(opened).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();
    let answer = |id| loop {
        let mail = serde_json::to_value(rx.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap();
        if mail["answer"]["id"].as_u64() == Some(id) {
            break mail["page"]["version"].as_u64().unwrap();
        }
    };
    let version = answer(1);
    host.submit(
        session,
        2,
        &key,
        merged,
        version,
        Some(&DiskToken::File(FileRev::from(conflict_rev.to_owned()))),
        &[EditKind::ReplacePage],
    )
    .unwrap();
    let version = answer(2);
    host.save_now(session, &[key.clone()]);
    assert_eq!(
        host.wait_published(
            session,
            &vec![(key, version, None)],
            Duration::from_secs(10)
        ),
        PageOperation::Applied
    );
    drop(host);
}

/// Every file under `dir` (graph root, including logseq/ and any trash) whose
/// bytes contain `needle`.
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if fs::read(&p)
                .map(|b| String::from_utf8_lossy(&b).contains(needle))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
    out
}

/// L9-03: a decision map that omits a theirs-only row. The merge must keep the
/// block or refuse; it must not silently drop it. Fails on og 709f5b67f:
/// `decision_for` defaults to Mine and the theirs-only block is gone.
#[test]
fn l9_03_a_missing_decision_never_drops_a_theirs_only_block() {
    let _guard = crate::integration_lock();
    let root = scratch("l9-03");
    fs::write(root.join(PAGE), BASE).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, "the plan for tomorrow");
    external(&root, &format!("{BASE}- {THEIRS_ONLY}\n"));
    let diff = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &[]).unwrap();
    assert!(
        diff.rows.iter().any(|r| r.kind == RowKind::Removed
            && r.theirs
                .as_ref()
                .is_some_and(|t| t.text.contains(THEIRS_ONLY))),
        "the review shows the external block as a theirs-only row"
    );
    let merged = merge_live_conflict(
        &store,
        PAGE,
        &page,
        &diff.conflict_rev,
        None,
        &[],
        &HashMap::new(),
        "union",
    );
    let kept = match &merged {
        Ok(dto) => dto.blocks.iter().any(|b| b.raw.contains(THEIRS_ONLY)),
        Err(_) => true, // refusing is an acceptable outcome
    };
    let _ = fs::remove_dir_all(&root);
    assert!(
        kept,
        "missing decision silently dropped the theirs-only block: {:?}",
        merged.map(|d| d.blocks.iter().map(|b| b.raw.clone()).collect::<Vec<_>>())
    );
}

/// L9-03 control: the UI's seeded decisions, 3-way review (ledger base), merge
/// without a "merged" row (so the merge aligns 2-way). The ids the review
/// published must still cover the theirs-only block.
#[test]
fn l9_03_control_seeded_three_way_decisions_keep_a_theirs_only_block() {
    let _guard = crate::integration_lock();
    let root = scratch("l9-03c");
    fs::write(root.join(PAGE), BASE).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, "the plan for tomorrow");
    external(&root, &format!("{BASE}- {THEIRS_ONLY}\n"));
    let bases = vec![BASE.to_owned()];
    let diff = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &bases).unwrap();
    assert!(diff.three_way);
    let mut chosen = HashMap::new();
    seeded(&diff.rows, &mut chosen);
    let merged = merge_live_conflict(
        &store,
        PAGE,
        &page,
        &diff.conflict_rev,
        diff.merge_base_rev.as_deref(),
        &bases,
        &chosen,
        "union",
    )
    .unwrap();
    let raws: Vec<String> = merged.blocks.iter().map(|b| b.raw.clone()).collect();
    let _ = fs::remove_dir_all(&root);
    assert!(raws.iter().any(|r| r.contains(THEIRS_ONLY)), "{raws:?}");
    assert!(
        raws.iter().any(|r| r.contains("the plan for tomorrow")),
        "{raws:?}"
    );
}

/// L9-04: the user picks "mine" for every row (a mistaken keep-mine). The
/// external editor's version must survive somewhere recoverable in the graph
/// (a conflict-trash copy, as the VCS-marker and sync-copy resolves keep).
/// Fails on og 709f5b67f: after the guarded save no file under the graph root
/// holds the external text. (The Concord ledger in app data may still hold it
/// as one of K=2 retained texts; that is outside the graph, not user-visible,
/// and evicted by later saves; not exercised here.)
#[test]
fn l9_04_keep_mine_leaves_a_recoverable_copy_of_the_external_version() {
    let _guard = crate::integration_lock();
    let root = scratch("l9-04");
    fs::write(root.join(PAGE), BASE).unwrap();
    let store = open(&root);
    let (page, base_rev) = draft(&store, 1, "the plan for tomorrow");
    let external_text =
        "- the shared intro line\n- the plan for today, as the other editor wrote it\n";
    external(&root, external_text);
    let diff = live_conflict_diff(&store, PAGE, &page, Some(&base_rev), &[]).unwrap();
    let mut chosen = HashMap::new();
    seeded(&diff.rows, &mut chosen);
    for v in chosen.values_mut() {
        *v = "mine".into();
    }
    let merged = merge_live_conflict(
        &store,
        PAGE,
        &page,
        &diff.conflict_rev,
        None,
        &[],
        &chosen,
        "mine",
    )
    .unwrap();
    save_guarded(&store, &merged, &diff.conflict_rev);
    let live = fs::read_to_string(root.join(PAGE)).unwrap();
    assert!(live.contains("the plan for tomorrow"), "{live}");
    let copies = files_containing(&root, "as the other editor wrote it");
    let _ = fs::remove_dir_all(&root);
    assert!(
        !copies.is_empty(),
        "the external version left no recoverable copy under the graph root"
    );
}
