//! GF3/D4: compare the existing single and general writers on identical,
//! disposable fixtures. Run alone: cost counters are process-global.
use super::*;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tine_store::cost_counters::{self, Counts};

fn measure(blocks: usize, referrers: usize, multi: bool) -> (Counts, Duration) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    let body = "- the target\n".repeat(blocks);
    let links = "- links [[Target]]\n".repeat(blocks);
    fs::write(root.join("pages/Target.md"), &body).unwrap();
    for index in 0..referrers {
        fs::write(root.join(format!("pages/Ref{index:05}.md")), &links).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let host = PageHost::start_for_tests(&store, &temp.path().join("app")).unwrap();
    assert!(host.quiesce_for_tests(Duration::from_secs(60)));

    // Exclude graph/host startup; include every operation write through
    // publication and cleanup, including first-use trash directories.
    cost_counters::reset();
    let started = Instant::now();
    let report = if multi {
        let plan = || plan_rename(&store, "Target", "Renamed", None, None, &|| {});
        crate::retry_on_conflict("page changed repeatedly during rename", || {
            let first = plan()?;
            assert!(first.single(&store)?.is_some(), "fixture is a single move");
            multi_rename(&store, &host, "Target", first, &plan, &[], true)
        })
    } else {
        rename_or_merge_page(&store, &host, "Target", "Renamed", None, None, &[])
    }
    .unwrap();
    let call = started.elapsed();
    assert!(host.quiesce_for_tests(Duration::from_secs(60)));
    let counts = cost_counters::snapshot();

    assert_eq!(report.outcome, RenameOutcome::Renamed);
    assert_eq!(report.touched.len(), referrers + 1);
    assert!(!root.join("pages/Target.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/Renamed.md")).unwrap(),
        body
    );
    for index in 0..referrers {
        assert_eq!(
            fs::read_to_string(root.join(format!("pages/Ref{index:05}.md"))).unwrap(),
            links.replace("[[Target]]", "[[Renamed]]")
        );
    }
    fn no_files(path: &std::path::Path) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            assert!(path.is_dir(), "unretired draft: {path:?}");
            no_files(&path);
        }
    }
    no_files(&temp.path().join("app/drafts-v2"));
    drop(host);
    store.close();
    (counts, call)
}

#[test]
#[ignore = "isolated GF3 measurement: run --lib rename_cost_tests::single_vs_multi_cost_gate -- --ignored --exact --nocapture with the full module name"]
fn single_vs_multi_cost_gate() {
    // Alternate order to reduce warm-cache bias; report raw call timings and
    // their median. This probe prints the gate outcome rather than turning
    // noisy wall-clock timing into a correctness assertion.
    const SAMPLES: usize = 7;
    let mut increases = Vec::new();
    println!(
        "D4 platform={} samples={SAMPLES} boundary=call-through-cleanup latency=call-median",
        std::env::consts::OS
    );
    println!("D4 blocks refs path files bytes fsyncs median_call_us raw_call_us");
    for blocks in [1, 60] {
        for referrers in [0, 1, 10] {
            let mut samples = [Vec::new(), Vec::new()];
            for round in 0..SAMPLES {
                for path in [round % 2, 1 - round % 2] {
                    samples[path].push(measure(blocks, referrers, path == 1));
                }
            }
            let costs = samples.map(|runs| {
                let counts = runs[0].0;
                let raw: Vec<_> = runs.iter().map(|(_, call)| call.as_micros()).collect();
                for (other, _) in &runs {
                    assert_eq!(other.files_written, counts.files_written);
                    assert_eq!(other.bytes_written, counts.bytes_written);
                    assert_eq!(other.fsyncs, counts.fsyncs);
                }
                let mut times = raw.clone();
                times.sort_unstable();
                (counts, times[SAMPLES / 2], raw)
            });
            for (path, (counts, median, raw)) in ["special", "multi"].into_iter().zip(&costs) {
                println!(
                    "D4 {blocks} {referrers} {path} {} {} {} {median} {raw:?}",
                    counts.files_written, counts.bytes_written, counts.fsyncs
                );
            }
            let (single, single_time, _) = &costs[0];
            let (multi, multi_time, _) = &costs[1];
            for (metric, before, after) in [
                (
                    "files",
                    single.files_written as u128,
                    multi.files_written as u128,
                ),
                (
                    "bytes",
                    single.bytes_written as u128,
                    multi.bytes_written as u128,
                ),
                ("fsyncs", single.fsyncs as u128, multi.fsyncs as u128),
                ("median_call_us", *single_time, *multi_time),
            ] {
                if after > before {
                    increases.push(format!(
                        "{blocks}b/{referrers}r {metric}: {before} -> {after}"
                    ));
                }
            }
        }
    }
    println!(
        "D4 gate={} increases={increases:?}",
        if increases.is_empty() { "PASS" } else { "STOP" }
    );
}
