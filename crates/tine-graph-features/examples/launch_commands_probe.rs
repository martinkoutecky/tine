//! GH #623 diagnosis probe: what the launch-path commands cost on a graph,
//! as wall time and as counts of file reads (a count is portable to Windows
//! with Defender, where each unseen-file open is ~250x slower; a time on
//! Linux is not). One process per run, numbers only.
//!
//! usage: launch_commands_probe <cold|write|warm> <graph copy> [checkpoint]
//!   cold   open with no checkpoint, run the commands
//!   write  open with a checkpoint path, run the commands, wait for the
//!          idle publisher's lazy-state re-checkpoint (`LAZY_POLL`) or force one
//!   warm   open from the checkpoint written earlier, run the commands
//! Needs `--features tine-store/test-faults` (dev-dependency enables it).
use std::path::{Path, PathBuf};
use std::time::Instant;
use tine_graph_features::conflicts::{conflict_inventory, ConflictQueue};
use tine_store::{cost_counters, OpenOptions, Store, WatchMode};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn counted<T>(
    label: &str,
    out: &mut serde_json::Map<String, serde_json::Value>,
    f: impl FnOnce() -> T,
) -> T {
    cost_counters::reset();
    let t = Instant::now();
    let value = f();
    let took = ms(t);
    let c = cost_counters::snapshot();
    out.insert(
        label.to_owned(),
        serde_json::json!({
            "ms": took,
            "storeReads": c.store_reads,
            "loadReads": c.full_reads,
            "preambleReads": c.preamble_reads,
            "hashReads": c.hash_reads,
            "stampsByPath": c.stamps_by_path,
            "readdirs": c.readdir,
            "parses": c.parses,
        }),
    );
    value
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a[1].as_str();
    let root = Path::new(&a[2]);
    let checkpoint: Option<PathBuf> =
        (mode != "cold").then(|| std::path::absolute(&a[3]).expect("checkpoint path"));
    let began = Instant::now();
    let (store, _, _) = Store::open(
        root,
        OpenOptions {
            watch: WatchMode::Poll,
            launch_checkpoint: checkpoint,
            ..Default::default()
        },
    )
    .expect("open");
    while !store.is_graph_ready().expect("ready") {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let ready_ms = ms(began);
    let mut out = serde_json::Map::new();
    // The frontend's launch calls, in the order it makes them.
    let view = counted("wholeGraph", &mut out, || {
        store.whole_graph().expect("graph")
    });
    counted("pageInventory_first", &mut out, || view.inventory());
    counted("pageInventory_second", &mut out, || view.inventory());
    let queue = ConflictQueue::default();
    counted("conflictInventory_first", &mut out, || {
        queue.inventory(&store).unwrap()
    });
    counted("conflictInventory_second", &mut out, || {
        queue.inventory(&store).unwrap()
    });
    counted("conflictInventory_walk", &mut out, || {
        conflict_inventory(&store).unwrap()
    });
    // The focus-return rescan (stat diff) on the unchanged graph.
    counted("scanRefresh_unchanged", &mut out, || {
        store.scan_refresh().unwrap()
    });
    counted("scanRefresh_unchanged_2", &mut out, || {
        store.scan_refresh().unwrap()
    });
    // A page click is `get_page`: `whole_graph` then `Store::page`, which takes
    // the store's writer lock. Time it while that lock is busy with a focus
    // rescan, and (write mode) with a checkpoint capture.
    let names: Vec<String> = std::fs::read_dir(root.join("pages"))
        .expect("pages")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".md").map(str::to_owned))
        .filter(|n| !n.contains('%') && !n.contains("___"))
        .take(40)
        .collect();
    let click = |i: usize| {
        let t = Instant::now();
        let _ = tine_graph_features::pages::get_page(
            &store,
            &names[i % names.len()],
            tine_core::model::PageKind::Page,
        );
        ms(t)
    };
    let stats = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        serde_json::json!({ "n": v.len(), "p50": v[v.len() / 2], "max": v[v.len() - 1] })
    };
    let idle: Vec<f64> = (0..40).map(click).collect();
    out.insert("pageClick_idle".into(), stats(idle));
    let busy = |label: &str,
                out: &mut serde_json::Map<String, serde_json::Value>,
                work: &(dyn Fn() + Sync)| {
        let done = std::sync::atomic::AtomicBool::new(false);
        let mut times = Vec::new();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                work();
                done.store(true, std::sync::atomic::Ordering::Release);
            });
            let mut i = 0;
            let origin = Instant::now();
            while !done.load(std::sync::atomic::Ordering::Acquire) {
                let at = ms(origin);
                let took = click(i);
                if took > 100.0 {
                    eprintln!("{label}: click {i} at +{at:.0} ms waited {took:.0} ms");
                }
                times.push(took);
                i += 1;
            }
        });
        out.insert(label.into(), stats(times));
    };
    busy("pageClick_during_8_rescans", &mut out, &|| {
        let origin = Instant::now();
        for n in 0..8 {
            let t = Instant::now();
            store.scan_refresh().unwrap();
            eprintln!("rescan {n} at +{:.0} ms took {:.0} ms", ms(origin), ms(t));
        }
    });
    if mode == "write" {
        let written = counted("checkpointWrite", &mut out, || store.write_checkpoint_now());
        eprintln!("checkpoint outcome: {written:?}");
        busy("pageClick_during_checkpoint", &mut out, &|| {
            store.write_checkpoint_now();
        });
        out.insert(
            "diagnostics_checkpoint".into(),
            store.diagnostics()["checkpoint"].clone(),
        );
        out.insert(
            "diagnostics_fullDiffs".into(),
            store.diagnostics()["fullDiffs"].clone(),
        );
    }
    println!(
        "{}",
        serde_json::json!({ "mode": mode, "readyMs": ready_ms, "commands": out })
    );
    store.close();
}
