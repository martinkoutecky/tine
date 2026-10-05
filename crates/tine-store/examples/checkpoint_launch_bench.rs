//! ADR 0070 measurements: one launch per process.
//! Usage: cargo run --release -p tine-store --features test-faults --example checkpoint_launch_bench --
//!        <cold|write|warm> <graph copy> <checkpoint file>
//! `cold` opens with no checkpoint; `write` opens, reaches Ready and writes the
//! checkpoint; `warm` opens from it. Prints one JSON line: open → first page
//! read (`page()` returns) and open → Ready in ms, checkpoint diagnostics and
//! VmRSS after Ready, then the first Ctrl-K search (`search`, the Quick
//! Switcher's `run_graph_search` call), the first `((` block search
//! (`find_blocks`) and the first backlinks of two pages after Ready.
//! `write` runs the searches and the first page's backlinks before writing, so
//! a checkpoint that keeps memos and lazy indexes carries them; the second
//! page's backlinks stay unasked. Run it on a COPY of a graph.

use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;
use tine_store::{Cancel, OpenOptions, PageId, SearchRequest, Store, WholeGraph};

/// A generic Ctrl-K needle; only hit counts are printed, never content.
const SEARCH: &str = "the";

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

/// The Quick Switcher's Ctrl-K call (`run_graph_search`: page names and
/// block text, 100 + 100 hits), as the app issues it.
fn ctrl_k(view: &WholeGraph) -> usize {
    let request = SearchRequest {
        text: SEARCH.to_owned(),
        within: None,
        page_limit: 100,
        block_limit: 100,
        explain: false,
        page_match_scope: None,
        page_view: None,
        block_view: None,
    };
    view.search(&request, &Cancel(Arc::new(AtomicBool::new(false))))
        .map(|execution| execution.hits.len())
        .unwrap_or(0)
}

/// The first Ctrl-K search (then the same search again, the in-memory floor),
/// the first `((` block search, and both backlinks reads, timed one by one.
fn timed_reads(view: &WholeGraph, first: &str, second: &str) -> serde_json::Value {
    let began = Instant::now();
    let ctrl_k_hits = ctrl_k(view);
    let ctrl_k_ms = ms(began);
    let began = Instant::now();
    ctrl_k(view);
    let ctrl_k_again_ms = ms(began);
    let began = Instant::now();
    let hits = view
        .find_blocks(SEARCH, 100, &Cancel(Arc::new(AtomicBool::new(false))))
        .map(|groups| groups.len())
        .unwrap_or(0);
    let search_ms = ms(began);
    let began = Instant::now();
    let first_groups = view.backlinks(first).map(|g| g.len()).unwrap_or(0);
    let first_ms = ms(began);
    let began = Instant::now();
    let second_groups = view.backlinks(second).map(|g| g.len()).unwrap_or(0);
    let second_ms = ms(began);
    serde_json::json!({
        "ctrlKMs": ctrl_k_ms, "ctrlKAgainMs": ctrl_k_again_ms, "ctrlKHits": ctrl_k_hits,
        "searchMs": search_ms, "searchGroups": hits,
        "backlinksMs": first_ms, "backlinksGroups": first_groups,
        "otherBacklinksMs": second_ms, "otherBacklinksGroups": second_groups,
    })
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// The two most `[[linked]]` names that are page files, counted over the
/// files before opening (a bench heuristic: the backlinks a user is likely to
/// open first).
fn most_linked(root: &Path) -> (String, String) {
    let pages: std::collections::HashSet<String> = page_files(root)
        .iter()
        .map(|file| file.trim_end_matches(".md").to_lowercase())
        .collect();
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for dir in ["pages", "journals"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            for part in text.split("[[").skip(1) {
                if let Some((name, _)) = part.split_once("]]") {
                    *counts.entry(name.to_lowercase()).or_default() += 1;
                }
            }
        }
    }
    let mut ranked: Vec<_> = counts
        .into_iter()
        .filter(|(name, _)| pages.contains(name))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    (ranked[0].0.clone(), ranked[1].0.clone())
}

/// The Markdown page files by name, chosen before opening.
fn page_files(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("pages"))
        .expect("pages directory")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".md"))
        .collect();
    names.sort();
    names
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, mode, root, checkpoint] = args.as_slice() else {
        panic!("usage: checkpoint_launch_bench <cold|write|warm> <graph copy> <checkpoint>");
    };
    let root = Path::new(root);
    let files = page_files(root);
    let page = PageId::from(format!("pages/{}", files.first().expect("a page")));
    let (first, second) = most_linked(root);
    let checkpoint = (mode != "cold").then(|| PathBuf::from(checkpoint));
    let began = Instant::now();
    let (store, _, _) = Store::open(
        root,
        OpenOptions {
            launch_checkpoint: checkpoint,
            ..Default::default()
        },
    )
    .expect("open graph");
    store.page(&page).expect("read first page");
    let first_page_ms = began.elapsed().as_secs_f64() * 1e3;
    while !store.is_graph_ready().expect("ready") {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let view = store.whole_graph().expect("ready");
    let ready_ms = began.elapsed().as_secs_f64() * 1e3;
    let pages = view.corpus().pages.len();
    let rss = rss_kib();
    let reads = (mode != "write").then(|| timed_reads(&view, &first, &second));
    let write = (mode == "write").then(|| {
        timed_reads(&view, &first, &first);
        format!("{:?}", store.write_checkpoint_now())
    });
    drop(view);
    let diag = store.diagnostics();
    println!(
        "{}",
        serde_json::json!({
            "mode": mode,
            "pages": pages,
            "firstPageMs": first_page_ms,
            "readyMs": ready_ms,
            "rssKiB": rss,
            "reads": reads,
            "write": write,
            "checkpoint": diag["checkpoint"],
        })
    );
    store.close();
}
