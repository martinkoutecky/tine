//! GH #623 item 3 probe: what one collapse toggle costs (I-25), on a page
//! with one parent block and on a 60-block page, in a real-sized graph.
//!
//! usage: collapse_toggle_probe <graph copy> <checkpoint>
//! Adds `pages/collapse_probe_1.md` and `pages/collapse_probe_60.md` before
//! opening and removes them at the end. Needs `--features tine-store/test-faults`.
use std::path::{Path, PathBuf};
use std::time::Instant;
use tine_core::model::PageKind;
use tine_graph_features::pages::{get_page, save_page};
use tine_store::{cost_counters, EditKind, OpenOptions, QueryDialect, Store, WatchMode};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn page_text(parents: usize) -> String {
    (0..parents)
        .map(|i| format!("- parent {i} [[collapse probe target]]\n  - child {i}\n"))
        .collect()
}

/// Warm the derived answers a real session holds: backlinks and a query.
fn warm(store: &Store) -> (f64, f64) {
    let view = store.whole_graph().unwrap();
    let t = Instant::now();
    let _ = view.backlinks("collapse probe target");
    let backlinks = ms(t);
    let t = Instant::now();
    let _ = view.query("(task TODO DOING)", QueryDialect::Simple);
    (backlinks, ms(t))
}

fn toggle(store: &Store, name: &str, on: bool) -> serde_json::Value {
    let read = get_page(store, name, PageKind::Page).unwrap().unwrap();
    let mut doc = read.doc.clone();
    let block = &mut doc.blocks[0];
    block.raw = if on {
        format!("{}\ncollapsed:: true", block.raw)
    } else {
        block.raw.replace("\ncollapsed:: true", "")
    };
    block.collapsed = on;
    cost_counters::reset();
    let t = Instant::now();
    let outcome = save_page(
        store,
        EditKind::SaveBlock,
        &read.id,
        &doc,
        Some(String::from(read.rev.clone())),
        false,
    )
    .unwrap();
    let save_ms = ms(t);
    let c = cost_counters::snapshot();
    let (backlinks_ms, query_ms) = warm(store);
    serde_json::json!({
        "page": name, "on": on, "outcome": format!("{outcome:?}").chars().take(40).collect::<String>(),
        "saveMs": save_ms, "backlinksAfterMs": backlinks_ms, "queryAfterMs": query_ms,
        "parses": c.parses, "fullReads": c.full_reads, "bytesWritten": c.bytes_written,
        "filesWritten": c.files_written, "fsyncs": c.fsyncs, "snapshotMs": c.snapshot_nanos as f64 / 1e6,
        "snapshotRebuilds": c.snapshot_rebuilds, "memoPageProbes": c.memo_page_probes,
        "queryFactsDerived": c.query_facts_derived, "queryCarryBlockProbes": c.query_carry_block_probes,
        "queryRegistryPagesRead": c.query_registry_pages_read, "cachePageCopies": c.cache_page_copies,
        "signatureBlockProbes": c.signature_block_probes, "hashReads": c.hash_reads,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let checkpoint = PathBuf::from(&args[2]);
    let files = [
        (root.join("pages/collapse_probe_1.md"), page_text(1)),
        (root.join("pages/collapse_probe_60.md"), page_text(30)),
    ];
    for (path, text) in &files {
        std::fs::write(path, text).unwrap();
    }
    let store = Store::open(
        Path::new(&root),
        OpenOptions {
            watch: WatchMode::Poll,
            launch_checkpoint: Some(checkpoint),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    store.whole_graph().unwrap();
    let (backlinks_ms, query_ms) = warm(&store);
    let (backlinks_warm_ms, query_warm_ms) = warm(&store);
    // The durable write alone: temp + fsync + rename + directory fsync.
    let t = Instant::now();
    for _ in 0..4 {
        let tmp = root.join("pages/.collapse_probe_tmp");
        std::fs::write(&tmp, page_text(30)).unwrap();
        std::fs::File::open(&tmp).unwrap().sync_all().unwrap();
        std::fs::rename(&tmp, root.join("logseq/.collapse_probe_tmp")).unwrap();
        std::fs::File::open(root.join("logseq"))
            .unwrap()
            .sync_all()
            .unwrap();
        std::fs::remove_file(root.join("logseq/.collapse_probe_tmp")).unwrap();
    }
    let durable_write_ms = ms(t) / 4.0;
    let mut rows = Vec::new();
    for name in ["collapse_probe_1", "collapse_probe_60"] {
        for on in [true, false, true, false] {
            rows.push(toggle(&store, name, on));
        }
    }
    let checkpoint = serde_json::json!({"checkpoint": store.diagnostics()["checkpoint"].clone(), "saves": store.diagnostics()["saves"].clone()});
    println!(
        "{}",
        serde_json::json!({
            "coldBacklinksMs": backlinks_ms, "coldQueryMs": query_ms,
            "warmBacklinksMs": backlinks_warm_ms, "warmQueryMs": query_warm_ms,
            "durableWriteMs": durable_write_ms, "toggles": rows, "checkpoint": checkpoint,
        })
    );
    store.close();
    for (path, _) in &files {
        let _ = std::fs::remove_file(path);
    }
}
