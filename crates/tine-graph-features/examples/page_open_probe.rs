//! GH #623 BR3 probe: what a page open costs right after a launch and after
//! the graph's file set changes, as wall time and as counts of file opens
//! (a count is portable to Windows with Defender, where each open is far
//! slower; a Linux time is not). One process per run, numbers only.
//!
//! usage: page_open_probe <write|warm|cold> <graph copy> <checkpoint> [concurrent]
//!   write  open with the checkpoint path, reach Ready, write it, close
//!   warm   open from the checkpoint; `concurrent` (default 4) page opens
//!          are issued at once, as a restored session does
//!   cold   the same with no checkpoint
//! Every open goes through `pages::get_page` (the app's `get_page`).
//! Needs `--features tine-store/test-faults` (the dev-dependency enables it).
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tine_core::model::PageKind;
use tine_graph_features::pages::get_page;
use tine_store::{cost_counters, OpenOptions, Store, WatchMode};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn open(root: &Path, checkpoint: Option<PathBuf>) -> Store {
    Store::open(
        root,
        OpenOptions {
            watch: WatchMode::Poll,
            launch_checkpoint: checkpoint,
            ..Default::default()
        },
    )
    .expect("open")
    .0
}

/// Page names chosen from file names before opening (no content read).
fn names(root: &Path, n: usize) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(root.join("pages"))
        .expect("pages")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| f.ends_with(".md") && !f.contains('%') && !f.contains("___"))
        .collect();
    files.sort();
    let step = (files.len() / (n + 1)).max(1);
    (1..=n)
        .filter_map(|i| files.get(i * step))
        .map(|f| f.trim_end_matches(".md").to_owned())
        .collect()
}

fn wait_ready(store: &Store) {
    while !store.is_graph_ready().unwrap_or(true) {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn opens(store: &Arc<Store>, names: &[String], began: Instant) -> serde_json::Value {
    cost_counters::reset();
    let handles: Vec<_> = names
        .iter()
        .cloned()
        .map(|name| {
            let store = Arc::clone(store);
            std::thread::spawn(move || {
                let t = Instant::now();
                let found = get_page(&store, &name, PageKind::Page)
                    .map(|r| r.is_some())
                    .unwrap_or(false);
                (ms(t), ms(began), found)
            })
        })
        .collect();
    let rows: Vec<_> = handles
        .into_iter()
        .map(|h| {
            let (took, at, found) = h.join().unwrap();
            serde_json::json!({"ms": took, "doneAtMs": at, "found": found})
        })
        .collect();
    let c = cost_counters::snapshot();
    serde_json::json!({"opens": rows, "preambleReads": c.preamble_reads, "fullReads": c.full_reads})
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args[1].as_str();
    let root = PathBuf::from(&args[2]);
    let checkpoint = PathBuf::from(&args[3]);
    let concurrent: usize = args.get(4).and_then(|n| n.parse().ok()).unwrap_or(4);
    let names = names(&root, concurrent + 2);
    // Windows-like per-file open cost (µs), e.g. 250.
    if let Some(delay) = std::env::var("PROBE_PREAMBLE_DELAY_US")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        cost_counters::set_preamble_open_delay(delay);
    }
    match mode {
        "write" => {
            let store = open(&root, Some(checkpoint));
            wait_ready(&store);
            println!("{:?}", store.write_checkpoint_now());
            store.close();
        }
        "warm" | "cold" => {
            let began = Instant::now();
            let store = Arc::new(open(&root, (mode == "warm").then_some(checkpoint)));
            // The session restore: several page opens issued at launch.
            let launch = opens(&store, &names[..concurrent], began);
            wait_ready(&store);
            let ready_ms = ms(began);
            let after_ready = opens(&store, &names[concurrent..concurrent + 1], began);
            // A new file (today's journal, a created page) changes the file set.
            let created = root.join("pages").join("page_open_probe_new.md");
            std::fs::write(&created, "- probe\n").unwrap();
            store.refresh(tine_store::Depth::Stamps).unwrap();
            let after_create = opens(&store, &names[concurrent + 1..], began);
            let _ = std::fs::remove_file(&created);
            println!(
                "{}",
                serde_json::json!({
                    "mode": mode, "readyMs": ready_ms,
                    "launch": launch, "afterReady": after_ready, "afterCreate": after_create,
                })
            );
            store.close();
        }
        "idle" => {
            // The launch's own file reads, with no page open issued.
            cost_counters::reset();
            let began = Instant::now();
            let store = open(&root, Some(checkpoint));
            wait_ready(&store);
            let c = cost_counters::snapshot();
            println!(
                "{}",
                serde_json::json!({"readyMs": ms(began), "preambleReads": c.preamble_reads, "fullReads": c.full_reads})
            );
            store.close();
        }
        _ => panic!("mode"),
    }
}
