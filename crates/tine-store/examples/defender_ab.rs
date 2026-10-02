//! THROWAWAY (GH #623 Defender A/B): one phase per process, JSON on stdout.
//! Usage: defender_ab <cold|warm|prims> <graph copy> [checkpoint file]
//!   cold  open with no checkpoint: first page, Ready, get_page (largest, median),
//!         Ctrl-K, scan_refresh x3 (the focus-return full freshness diff), then
//!         write the checkpoint.
//!   warm  open from that checkpoint: first page, Ready, then the same reads.
//!   prims raw per-file primitives, first touch (list, stat all, open all,
//!         read all) then second touch (read all again); no Store.
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;
use tine_store::{Cancel, OpenOptions, PageId, SearchRequest, Store, WholeGraph};

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

fn files(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    for dir in ["pages", "journals"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                out.push((path, len));
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn page_id(root: &Path, path: &Path) -> PageId {
    let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
    PageId::from(rel)
}

fn ctrl_k(view: &WholeGraph) -> (f64, usize) {
    let request = SearchRequest {
        text: "the".to_owned(),
        within: None,
        page_limit: 100,
        block_limit: 100,
        explain: false,
        page_match_scope: None,
        page_view: None,
        block_view: None,
    };
    let began = Instant::now();
    let hits = view
        .search(&request, &Cancel(Arc::new(AtomicBool::new(false))))
        .map(|e| e.hits.len())
        .unwrap_or(0);
    (ms(began), hits)
}

fn prims(root: &Path) -> serde_json::Value {
    let began = Instant::now();
    let list = files(root); // read_dir + one metadata per file (sizes), sorted
    let list_and_stat_ms = ms(began);
    let began = Instant::now();
    let mut mt = 0u64;
    for (p, _) in &list {
        if let Ok(m) = std::fs::metadata(p) {
            mt = mt.wrapping_add(m.len());
        }
    }
    let stat_ms = ms(began);
    let began = Instant::now();
    let mut opened = 0usize;
    for (p, _) in &list {
        if std::fs::File::open(p).is_ok() {
            opened += 1;
        }
    }
    let open_ms = ms(began);
    let began = Instant::now();
    let mut bytes = 0usize;
    for (p, _) in &list {
        bytes += std::fs::read(p).map(|b| b.len()).unwrap_or(0);
    }
    let read_first_ms = ms(began);
    let began = Instant::now();
    for (p, _) in &list {
        let _ = std::fs::read(p);
    }
    let read_second_ms = ms(began);
    serde_json::json!({
        "files": list.len(), "listAndStatMs": list_and_stat_ms, "statMs": stat_ms, "openMs": open_ms,
        "readFirstMs": read_first_ms, "readSecondMs": read_second_ms, "bytes": bytes,
        "opened": opened, "sum": mt,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args[1].as_str();
    let root = Path::new(&args[2]);
    if mode == "prims" {
        println!("{}", serde_json::json!({ "mode": mode, "prims": prims(root) }));
        return;
    }
    let checkpoint = PathBuf::from(&args[3]);
    let list = files(root);
    let largest = page_id(root, &list.last().unwrap().0);
    let median = page_id(root, &list[list.len() / 2].0);
    let first = page_id(root, &list[0].0);
    let began = Instant::now();
    let (store, _, _) = Store::open(
        root,
        OpenOptions { launch_checkpoint: Some(checkpoint.clone()), ..Default::default() },
    )
    .expect("open graph");
    store.page(&first).expect("first page");
    let first_page_ms = ms(began);
    while !store.is_graph_ready().expect("ready") {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let ready_ms = ms(began);
    let view = store.whole_graph().expect("ready");
    let pages = view.parsed_page_ids().len();
    let t = Instant::now();
    store.page(&largest).expect("largest");
    let page_largest_ms = ms(t);
    let t = Instant::now();
    store.page(&median).expect("median");
    let page_median_ms = ms(t);
    let (ctrl_k_ms, ctrl_k_hits) = ctrl_k(&view);
    drop(view);
    let mut scans = Vec::new();
    for _ in 0..3 {
        let t = Instant::now();
        store.scan_refresh().expect("scan_refresh");
        scans.push(ms(t));
    }
    let write = (mode == "cold").then(|| {
        let t = Instant::now();
        let r = format!("{:?}", store.write_checkpoint_now());
        serde_json::json!({ "ms": ms(t), "result": r.chars().take(200).collect::<String>() })
    });
    let diag = store.diagnostics();
    let launch = &diag["launch"];
    let pass = launch["loadPasses"].get(0).cloned().unwrap_or(serde_json::Value::Null);
    println!(
        "{}",
        serde_json::json!({
            "mode": mode, "pages": pages,
            "firstPageMs": first_page_ms, "readyMs": ready_ms,
            "pageLargestMs": page_largest_ms, "pageMedianMs": page_median_ms,
            "ctrlKMs": ctrl_k_ms, "ctrlKHits": ctrl_k_hits,
            "scanRefreshMs": scans, "checkpointWrite": write,
            "diagReadyMs": launch["readyMs"], "diagPublishMs": launch["publishMs"], "diagOpenMs": launch["openMs"],
            "diagBaselineWalk": launch["baselineWalk"], "diagFillRevs": launch["fillRevs"],
            "diagLoadPass": pass,
            "diagFullDiffs": diag["fullDiffs"], "checkpoint": diag["checkpoint"],
        })
    );
    store.close();
}
