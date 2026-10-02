//! GH #623 / storage spec §5.1 step 1, §7.6: a cold launch reads every graph
//! file once. The load pass records each file's stamp (taken before the read)
//! and the revision of the bytes it parsed, and the watcher baseline is built
//! from those observations: no second pass re-reads every file to hash it
//! (the old `fill_revs`), and no name walk re-opens every page preamble.
use std::fs;
use std::time::{Duration, SystemTime};
use tine_store::{cost_counters, OpenOptions, Store, WatchMode};

fn backdate(path: &std::path::Path) {
    // Outside the 2 s racy window (§5.4): a fresh fixture file is racy and is
    // legitimately re-read once by the launch diff.
    let old = SystemTime::now() - Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(old)
        .unwrap();
}

fn graph_with(pages: usize) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("pages")).unwrap();
    fs::create_dir_all(root.path().join("journals")).unwrap();
    fs::create_dir_all(root.path().join("logseq")).unwrap();
    let config = root.path().join("logseq/config.edn");
    fs::write(&config, "{:preferred-format :markdown}\n").unwrap();
    backdate(&config);
    for index in 0..pages {
        let path = match index % 4 {
            0 => root.path().join(format!("pages/Page {index}.md")),
            1 => root.path().join(format!("pages/titled{index}.md")),
            2 => root.path().join(format!("pages/Org {index}.org")),
            _ => {
                let date = tine_core::date::JournalDate::from_days(20_000 + index as i64);
                root.path()
                    .join("journals")
                    .join(format!("{}.md", date.file_stem()))
            }
        };
        let text = match index % 4 {
            1 => format!("title:: Titled {index}\n- links [[Page 0]] #tag{index}\n"),
            2 => format!("* heading {index}\n** child [[Page 0]]\n"),
            _ => format!("- block {index} [[Page 0]]\n  - child\n"),
        };
        fs::write(&path, text).unwrap();
        backdate(&path);
    }
    root
}

/// File content reads of one cold launch to Ready, plus its watcher baseline.
fn launch_reads(root: &std::path::Path) -> cost_counters::Counts {
    cost_counters::reset();
    let (store, _, _) = Store::open(
        root,
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap();
    let view = store.whole_graph().unwrap();
    let counts = cost_counters::snapshot();
    drop(view);
    store.close();
    counts
}

fn reads(counts: &cost_counters::Counts) -> u64 {
    counts.full_reads + counts.preamble_reads + counts.hash_reads
}

#[test]
fn a_cold_launch_reads_each_graph_file_once() {
    // The config-only graph measures the fixed reads of config.edn itself.
    let empty = graph_with(0);
    let fixed = launch_reads(empty.path());
    const PAGES: usize = 40;
    let graph = graph_with(PAGES);
    let counts = launch_reads(graph.path());
    let per_graph = reads(&counts) - reads(&fixed);
    assert_eq!(
        per_graph, PAGES as u64,
        "GH #623 / spec §7.6: a cold launch must read each of the {PAGES} graph files once \
         (load pass records stamp + rev; the watcher baseline is built from it), \
         got {per_graph} file reads: full {} preamble {} hash {} (config-only graph: full {} preamble {} hash {}); \
         exemplar crates/tine-store/src/model.rs warm_page_cache_inner",
        counts.full_reads,
        counts.preamble_reads,
        counts.hash_reads,
        fixed.full_reads,
        fixed.preamble_reads,
        fixed.hash_reads,
    );
}
