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
    store.close();
}
