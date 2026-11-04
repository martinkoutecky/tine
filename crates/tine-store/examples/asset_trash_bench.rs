//! Cost-model audit 2026-11-02 row 9a / row 5 measurements (lane
//! og-perf-asset-trash). Run on a COPY of a graph prepared with unreferenced
//! `assets/spareN.png` files, a referenced `assets/img0.png`, and a page
//! `PrintTarget` (embeds):
//!   cargo run --release -p tine-store --features test-faults --example asset_trash_bench -- <graph copy>
//! Prints one JSON line: per trash, its wall time, the time and the documents
//! the orphan check spent under the Store writer, and the latency of a page
//! save issued 2 ms after the trash began (it waits for the writer); then the
//! wall time and counters of one page's print flow (sheet inputs + document).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tine_store::{
    cost_counters, Area, EditKind, PageId, Refusal, SaveBase, Store, TrashIf, TxOutcome, Why,
};

use tine_store as store_crate;
#[path = "../src/test_host_writer.rs"]
mod test_host_writer;

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn trash(store: &Store, name: &str) -> &'static str {
    let id = store.file_id(Area::Assets, name).unwrap();
    let (_, rev) = store.read(&id, None).unwrap();
    let mut tx = store.transaction(None);
    tx.trash(&id, rev, TrashIf::UnreferencedAsset);
    match tx.commit() {
        TxOutcome::Committed { .. } => "trashed",
        TxOutcome::NotCommitted {
            why: Why::Refused(Refusal::AssetReferenced),
            ..
        } => "referenced",
        other => panic!("{other:?}"),
    }
}

fn save(store: &Store, writer: &test_host_writer::HostWriter, rel: &str, text: &str) -> Duration {
    let id = PageId::from(rel.to_string());
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[0].raw = text.to_owned();
    let started = Instant::now();
    writer
        .save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc,
        )
        .unwrap();
    started.elapsed()
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("graph copy"));
    let opened = Instant::now();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
    let open_ms = ms(opened.elapsed());
    let writer = test_host_writer::HostWriter::start(&store);
    let edited = "pages/PrintTarget2.md";
    let mut saves_alone = Vec::new();
    for round in 0..3 {
        saves_alone.push(ms(save(
            &store,
            &writer,
            edited,
            &format!("- alone {round}"),
        )));
    }
    let mut trashes = Vec::new();
    for (round, name) in [
        "spare0.png",
        "spare1.png",
        "spare2.png",
        "spare3.png",
        "spare4.png",
        "img0.png",
    ]
    .into_iter()
    .enumerate()
    {
        cost_counters::reset();
        let (concurrent, outcome, wall) = std::thread::scope(|scope| {
            let store = &store;
            let writer = &writer;
            let saver = scope.spawn(move || {
                std::thread::sleep(Duration::from_millis(2));
                save(store, writer, edited, &format!("- during trash {round}"))
            });
            let started = Instant::now();
            let outcome = trash(store, name);
            let wall = started.elapsed();
            (saver.join().unwrap(), outcome, wall)
        });
        let counts = cost_counters::snapshot();
        trashes.push(serde_json::json!({
            "asset": name,
            "outcome": outcome,
            "trashMs": ms(wall),
            "writerHeldCheckMs": counts.orphan_check_nanos as f64 / 1e6,
            "documentsUnderWriter": counts.orphan_check_documents,
            "documentsScanned": counts.asset_ref_documents,
            "concurrentSaveMs": ms(concurrent),
        }));
    }
    // Print flow of one page, as src/print.ts runs it.
    let mut prints = Vec::new();
    for _ in 0..3 {
        cost_counters::reset();
        let started = Instant::now();
        let _sheets = tine_graph_features::publish::sheet_export_inputs(
            &store,
            Some(&["PrintTarget".to_owned()]),
            None,
        )
        .unwrap();
        let inputs = started.elapsed();
        let html = tine_graph_features::print::page_print_html_with_sheets(
            &store,
            "PrintTarget",
            Default::default(),
            Vec::new(),
        )
        .unwrap()
        .unwrap();
        let counts = cost_counters::snapshot();
        assert!(html.contains("the printed page"));
        prints.push(serde_json::json!({
            "sheetInputsMs": ms(inputs),
            "totalMs": ms(started.elapsed()),
            "statsByPath": counts.stamps_by_path,
            "hashReads": counts.hash_reads,
            "corpusCopies": counts.corpus,
            "blockEmbedFound": html.contains("embed block-embed"),
        }));
    }
    println!(
        "{}",
        serde_json::json!({
            "openToReconciledMs": open_ms,
            "savesAloneMs": saves_alone,
            "trashes": trashes,
            "prints": prints,
        })
    );
    drop(writer);
    store.close();
}
