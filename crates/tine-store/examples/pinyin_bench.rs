//! Fixed page-search probes for GH #151. Run on private graph copies only.
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{Duration, Instant};
use tine_store::{Cancel, SearchRequest, Store};

fn main() {
    let root = std::env::args().nth(1).expect("graph copy path");
    let enabled = std::env::args().nth(2).as_deref() == Some("on");
    let start = Instant::now();
    let store = Store::open(std::path::Path::new(&root), Default::default())
        .unwrap()
        .0;
    let graph = store.whole_graph().unwrap();
    while !store.is_graph_ready().unwrap() {
        std::thread::sleep(Duration::from_millis(1));
    }
    let ready_ms = start.elapsed().as_secs_f64() * 1000.0;
    let graph = {
        drop(graph);
        store.whole_graph().unwrap()
    };
    let cache = graph.transliteration_cache_payload();
    let mut metrics = Vec::new();
    for query in ["beijing", "bj", "page"] {
        let mut times = Vec::new();
        let mut count = 0;
        for i in 0..101 {
            let start = Instant::now();
            count = graph
                .search(
                    &SearchRequest {
                        text: query.into(),
                        transliteration: enabled,
                        within: None,
                        page_limit: 100,
                        block_limit: 0,
                        explain: false,
                        page_match_scope: None,
                        page_view: None,
                        block_view: None,
                    },
                    &Cancel(Arc::new(AtomicBool::new(false))),
                )
                .unwrap()
                .hits
                .len();
            if i > 0 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        times.sort_by(f64::total_cmp);
        metrics.push(
            serde_json::json!({"probe":query,"p50_ms":times[50],"p95_ms":times[95],"hits":count}),
        );
    }
    // Never print graph names or contents.
    println!(
        "{}",
        serde_json::json!({"enabled": enabled,"ready_ms":ready_ms,"cache_names":cache.0,"cache_payload_bytes":cache.1,"metrics":metrics})
    );
    if std::env::args().nth(3).as_deref() == Some("edit") {
        // The caller supplies a disposable fixture. Do not print any names or
        // contents. Keep a prior snapshot held, as live editors/searches do.
        let inventory = graph.inventory(tine_store::InventoryScope::All);
        let id = inventory
            .0
            .iter()
            .find_map(|entry| match &entry.target {
                tine_store::Resolved::Existing { id, .. } => Some(id.clone()),
                _ => None,
            })
            .expect("a physical fixture page");
        let mut save_ms = Vec::new();
        let mut publication_ms = Vec::new();
        let mut last = tine_store::cost_counters::snapshot();
        for n in 0..31 {
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = format!("benchmark body {n}");
            tine_store::cost_counters::reset();
            let start = Instant::now();
            assert!(matches!(
                store.save(
                    tine_store::EditKind::ReplacePage,
                    &id,
                    tine_store::SaveBase::Existing(read.rev),
                    &doc
                ),
                tine_store::SaveOutcome::Saved(_)
            ));
            last = tine_store::cost_counters::snapshot();
            if n > 0 {
                save_ms.push(start.elapsed().as_secs_f64() * 1000.0);
                publication_ms.push(last.snapshot_nanos as f64 / 1e6);
            }
        }
        save_ms.sort_by(f64::total_cmp);
        publication_ms.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"edit_samples":30,"save_p50_ms":save_ms[15],"save_p95_ms":save_ms[28],"publication_p50_ms":publication_ms[15],"publication_p95_ms":publication_ms[28],"last_files_written":last.files_written,"last_fsyncs":last.fsyncs,"last_full_reads":last.full_reads,"last_tree_copies":last.shared_tree_node_copies})
        );
    }
    store.close();
}
