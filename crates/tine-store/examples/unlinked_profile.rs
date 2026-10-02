//! GH #623: where do `get_unlinked_refs` and the page open spend their time?
//! Usage: cargo run --release -p tine-store --example unlinked_profile -- <graph copy> <page>...
//! `UNLINKED_DUMP=<file>` appends the full results (for old-vs-new equality diffs).
//! Prints, per page and per run (3), the page-open (`resolve` + `Store::page`)
//! and the unlinked-references time; a second thread measures how long a tiny
//! writer-lock operation (`Store::save` of an untouched page) takes while the
//! unlinked scan runs, which answers "does the command hold the writer lock".
use std::path::Path;
use std::time::Instant;
use tine_store::{OpenOptions, Resolved, Store};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .expect("usage: unlinked_profile <graph copy> <page>...");
    let targets: Vec<String> = args.collect();
    let t = Instant::now();
    let (store, _, _) = Store::open(Path::new(&root), OpenOptions::default()).expect("open graph");
    let view = store.whole_graph().expect("initial load");
    println!("open+load {:?}", t.elapsed());
    for target in &targets {
        for run in 1..=3 {
            let t = Instant::now();
            let id = match view.resolve(target, false) {
                Resolved::Existing { id, .. } => Some(id),
                _ => None,
            };
            let resolve = t.elapsed();
            let t = Instant::now();
            let page = id.as_ref().map(|id| store.page(id).expect("read page"));
            let open = t.elapsed();
            let blocks = page.as_ref().map_or(0, |p| p.doc.blocks.len());
            // A fresh view per run so the derived memo does not answer.
            let fresh = store.whole_graph().unwrap();
            let t = Instant::now();
            let result = fresh.unlinked_references(target);
            let unlinked = t.elapsed();
            let (groups, rows) = match &result {
                Ok(groups) => (groups.len(), groups.iter().map(|g| g.blocks.len()).sum()),
                Err(error) => {
                    println!("  (refused: {error:?})");
                    (0, 0)
                }
            };
            if let (Some(path), Ok(groups)) = (std::env::var_os("UNLINKED_DUMP"), &result) {
                use std::io::Write;
                if run == 1 {
                    let mut file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .unwrap();
                    writeln!(file, "== {target:?}").unwrap();
                    for group in groups.iter() {
                        writeln!(file, "{group:?}").unwrap();
                    }
                }
            }
            println!(
                "{target:?} run{run}: resolve {resolve:?} page({blocks} top blocks) {open:?} unlinked {unlinked:?} ({groups} groups, {rows} rows)"
            );
        }
    }
    drop(view);
    store.close();
}
