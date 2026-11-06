//! Synthetic Ctrl+K scan A/B; output root is explicit and never a real graph.
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};
use tine_store::{Cancel, SearchRequest, Store};

fn main() {
    let output = std::env::args().nth(1).expect("artifact directory");
    for properties in [false, true] {
        let dir = tempfile::Builder::new()
            .prefix("search-bench-")
            .tempdir_in(&output)
            .unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        for page in 0..200 {
            let mut body = if properties {
                "category:: ordinary-page\n\n".to_owned()
            } else {
                String::new()
            };
            for block in 0..60 {
                body.push_str(if block == 59 {
                    "- Needle ordinary body\n"
                } else {
                    "- Ordinary text to search through\n"
                });
                if properties {
                    body.push_str("  reviewer:: Avery\n  id:: ignoredidentity\n");
                }
            }
            std::fs::write(dir.path().join(format!("pages/Page{page}.md")), body).unwrap();
        }
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let graph = store.whole_graph().unwrap();
        let request = SearchRequest {
            text: "Needle".into(),
            transliteration: false,
            within: None,
            page_limit: 0,
            block_limit: 100,
            explain: false,
            page_match_scope: None,
            page_view: None,
            block_view: None,
        };
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let mut samples = Vec::new();
        for _ in 0..31 {
            let started = Instant::now();
            let result = graph.search(&request, &cancel).unwrap();
            assert_eq!(result.hits.len(), 100);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let first = samples.remove(0);
        samples.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({ "pages": 200, "blocks": 12000, "properties": properties,
            "firstMs": first, "warmMedianMs": samples[15], "runs": samples.len() })
        );
        store.close();
    }
}
