//! C3W W4 (L03, I-11): resolving a sync conflict copy never writes one block
//! `id::` twice. A reorder conflict aligns the moved block as Added (winner)
//! plus Removed (copy) with the SAME id, and "keep both" pulled the copy in with
//! its id; an Org block's id lives in its `:PROPERTIES:` drawer as `:id:`, which
//! keep-both's id stripping did not see.
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::sync_diff::DiffRow;
use tine_graph_features::conflicts;
use tine_store::Store;

const ID: &str = "6679aaaa-0000-4000-8000-000000000001";

fn scratch() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3w-w4-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    root
}

fn all_rows(rows: &[DiffRow], decision: &str, out: &mut HashMap<String, String>) {
    for row in rows {
        out.insert(row.id.clone(), decision.into());
        all_rows(&row.children, decision, out);
    }
}

fn resolve_all(ext: &str, winner: &str, copy: &str, decision: &str) -> String {
    let root = scratch();
    let win = format!("pages/A.{ext}");
    let conf = format!("pages/A.sync-conflict-20260929-101010-ABCDEFG.{ext}");
    fs::write(root.join(&win), winner).unwrap();
    fs::write(root.join(&conf), copy).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let diff = conflicts::sync_conflict_diff(&store, &win, &conf, &[])
        .unwrap()
        .unwrap();
    let mut decisions = HashMap::new();
    all_rows(&diff.rows, decision, &mut decisions);
    conflicts::resolve_sync_conflict(
        &store,
        None,
        &win,
        &conf,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        None,
        &[],
        "mine",
    )
    .unwrap();
    let out = fs::read_to_string(root.join(&win)).unwrap();
    let _ = fs::remove_dir_all(&root);
    out
}

#[test]
fn w4_keep_both_on_a_reorder_conflict_writes_each_block_id_once() {
    let winner = format!("- moved\n  id:: {ID}\n- stays\n- stays too\n");
    let copy = format!("- stays\n- stays too\n- moved, edited there\n  id:: {ID}\n");
    for decision in ["both", "theirs"] {
        let merged = resolve_all("md", &winner, &copy, decision);
        assert!(
            merged.matches(ID).count() <= 1,
            "{decision}: duplicate id:: in\n{merged}"
        );
    }
    // Keep-theirs drops the winner's copy, so the conflict copy keeps the id
    // and references to it still resolve.
    let merged = resolve_all("md", &winner, &copy, "theirs");
    assert_eq!(merged.matches(ID).count(), 1, "{merged}");
    // Keep-both keeps the winner's identity and both bodies.
    let merged = resolve_all("md", &winner, &copy, "both");
    assert!(
        merged.contains(&format!("- moved\n  id:: {ID}")),
        "{merged}"
    );
    assert!(merged.contains("moved, edited there"), "{merged}");
}

#[test]
fn w4_keep_both_strips_an_org_drawer_id_from_the_copy() {
    let winner = format!("* mine text\n:PROPERTIES:\n:id: {ID}\n:END:\n");
    let copy = format!("* their text\n:PROPERTIES:\n:id: {ID}\n:END:\n");
    let merged = resolve_all("org", &winner, &copy, "both");
    assert_eq!(merged.matches(ID).count(), 1, "{merged}");
    assert!(merged.contains("their text"), "{merged}");
    assert!(
        !merged.contains(":PROPERTIES:\n:END:"),
        "no empty drawer left: {merged}"
    );
}
