//! GH #623 (Ellis build 4): the fail-before proof for `conflict_inventory`'s
//! per-file reads. A defender-ON Windows open of a file unseen since boot costs
//! ~250x a warm one (subagent-tasks/notes/2026-10-02-windows-defender-ab.md),
//! so the proof is a COUNT of complete file reads, not a timing: once the
//! graph is loaded, an unmarked graph's inventory reads no page file at all,
//! and a marked graph reads exactly its anchor-carrying files.
//!
//! The counters are process-global, so this file holds one test. Fixture files
//! are backdated out of the 2 s racy window (§5.4): a freshly written file is
//! legitimately re-hashed by the watcher's racy follow-up diff about 2 s after
//! open, and under load that follow-up landed inside the counted window
//! (15 or 63 extra hash reads, never a marker read).
//! Invariants: I-25 (unit cost), I-12.

use std::fs;
use std::time::{Duration, SystemTime};
use tine_graph_features::conflicts::conflict_inventory;
use tine_store::{cost_counters, Store};

fn backdate(path: &std::path::Path) {
    let old = SystemTime::now() - Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(old)
        .unwrap();
}

#[test]
fn the_inventory_reads_only_pages_that_may_carry_an_anchor_line() {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    fs::write(
        root.path().join("logseq/config.edn"),
        "{:preferred-format :markdown}\n",
    )
    .unwrap();
    const PAGES: usize = 60;
    for i in 0..PAGES {
        fs::write(
            root.path().join(format!("pages/Page {i}.md")),
            format!("- block {i}\n  - child\n"),
        )
        .unwrap();
    }
    // Two marked pages and one fenced anchor ("maybe": read, then not listed).
    for (name, text) in [
        ("M1", "- a\n<<<<<<< HEAD\n- b\n>>>>>>> x\n"),
        ("M2", ">>>>>>> x\n- b\n"),
        ("F1", "- a\n```\n<<<<<<< HEAD\n```\n"),
    ] {
        fs::write(root.path().join(format!("pages/{name}.md")), text).unwrap();
    }
    for dir in ["pages", "logseq"] {
        for entry in fs::read_dir(root.path().join(dir)).unwrap() {
            backdate(&entry.unwrap().path());
        }
    }
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    cost_counters::reset();
    let inventory = conflict_inventory(&store).unwrap();
    let counts = cost_counters::snapshot();
    assert_eq!(inventory.vcs_markers.len(), 2);
    let reads = counts.store_reads + counts.full_reads + counts.preamble_reads + counts.hash_reads;
    assert_eq!(
        reads, 5,
        "GH #623 / I-25: conflict_inventory must read only the pages whose load-time \
         bytes carried an anchor line (M1, M2 and the fenced F1: one marker scan \
         each, plus the queue object's own read of the two listed pages), never the \
         {PAGES} unmarked pages; got {reads} reads (store {} full {} preamble {} hash {}); the flag \
         is `Store::may_carry_vcs_markers`, exemplar crates/tine-store/src/model.rs `DiskObs`",
        counts.store_reads, counts.full_reads, counts.preamble_reads, counts.hash_reads,
    );
}
