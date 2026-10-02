//! ADR 0070 measurements: one launch per process.
//! Usage: cargo run --release -p tine-store --features test-faults --example checkpoint_launch_bench --
//!        <cold|write|warm> <graph copy> <checkpoint file>
//! `cold` opens with no checkpoint; `write` opens, reaches Ready and writes the
//! checkpoint; `warm` opens from it. Prints one JSON line: open → first page
//! read (`page()` returns) and open → Ready in ms, checkpoint diagnostics and
//! VmRSS after Ready. Run it on a COPY of a graph.

use std::path::{Path, PathBuf};
use std::time::Instant;
use tine_store::{OpenOptions, PageId, Store};

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

/// The first Markdown page by name, chosen before opening.
fn first_page(root: &Path) -> PageId {
    let mut names: Vec<String> = std::fs::read_dir(root.join("pages"))
        .expect("pages directory")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".md"))
        .collect();
    names.sort();
    PageId::from(format!("pages/{}", names.first().expect("a page")))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, mode, root, checkpoint] = args.as_slice() else {
        panic!("usage: checkpoint_launch_bench <cold|write|warm> <graph copy> <checkpoint>");
    };
    let root = Path::new(root);
    let page = first_page(root);
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
    let pages = view.parsed_page_ids().len();
    drop(view);
    let rss = rss_kib();
    let write = (mode == "write").then(|| format!("{:?}", store.write_checkpoint_now()));
    let diag = store.diagnostics();
    println!(
        "{}",
        serde_json::json!({
            "mode": mode,
            "pages": pages,
            "firstPageMs": first_page_ms,
            "readyMs": ready_ms,
            "rssKiB": rss,
            "write": write,
            "checkpoint": diag["checkpoint"],
        })
    );
    store.close();
}
