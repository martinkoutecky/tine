//! GH #623: first-call `unlinked_references` latency for every name in a file.
//! Usage: unlinked_sweep <graph copy> <names file> [dump file]
//! A fresh `whole_graph()` per name (so the per-view memo never answers); the
//! first name also pays any lazy per-generation structure. `dump file` gets the
//! full result of every name (for old-vs-new byte-equality diffs).
use std::io::Write;
use std::path::Path;
use std::time::Instant;
use tine_store::{OpenOptions, Store};

/// Process CPU time in microseconds: on this shared, heavily loaded machine
/// wall time is inflated by contention, CPU time much less so. Off Unix (the
/// Windows CI compiles examples) it falls back to wall time since first call.
#[cfg(unix)]
fn cpu_us() -> u128 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid out-pointer for the duration of the call.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    ts.tv_sec as u128 * 1_000_000 + ts.tv_nsec as u128 / 1000
}
#[cfg(not(unix))]
fn cpu_us() -> u128 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .expect("usage: unlinked_sweep <graph> <names> [dump]");
    let names = std::fs::read_to_string(args.next().expect("names file")).expect("read names");
    let mut dump = args
        .next()
        .map(|p| std::fs::File::create(p).expect("dump file"));
    let (store, _, _) = Store::open(Path::new(&root), OpenOptions::default()).expect("open");
    let _ = store.whole_graph().expect("initial load");
    let mut worst = 0u128;
    for name in names.lines().filter(|l| !l.is_empty()) {
        let view = store.whole_graph().unwrap();
        let t = Instant::now();
        let c0 = cpu_us();
        let result = view.unlinked_references(name);
        let ms = t.elapsed().as_micros();
        let cpu = cpu_us() - c0;
        worst = worst.max(ms);
        let (groups, rows) = match &result {
            Ok(g) => (g.len(), g.iter().map(|g| g.blocks.len()).sum::<usize>()),
            Err(_) => (0, 0),
        };
        println!(
            "{:>8.1} ms (cpu {:>7.1})  {groups:>5} groups {rows:>6} rows  {name:?}",
            ms as f64 / 1000.0,
            cpu as f64 / 1000.0
        );
        if let (Some(file), Ok(groups)) = (dump.as_mut(), &result) {
            writeln!(file, "== {name:?}").unwrap();
            for group in groups.iter() {
                writeln!(file, "{group:?}").unwrap();
            }
        }
    }
    println!("worst {:.1} ms", worst as f64 / 1000.0);
    store.close();
}
