//! G6 gate: open a graph, read every page, close — and write nothing.
//! Usage: cargo run -p tine-store --example open_untouched -- <graph copy>
//! Pair with scripts/og-g6-untouched.sh, which snapshots the tree around this run.

use std::path::Path;
use tine_store::{OpenOptions, Store};

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: open_untouched <graph copy>");
    let (store, _, _) = Store::open(Path::new(&root), OpenOptions::default()).expect("open graph");
    let graph = store.whole_graph().expect("initial load");
    let ids = graph.parsed_page_ids();
    for id in &ids {
        store.page(id).expect("read page");
    }
    println!(
        "opened and read {} pages, {} unreadable",
        ids.len(),
        graph.unreadable_files().len()
    );
    drop(graph);
    store.close();
}
