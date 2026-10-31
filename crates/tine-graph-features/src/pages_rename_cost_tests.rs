//! R5: measure the consolidated writer against recorded single/general costs on
//! disposable fixtures. Run alone: cost counters are process-global.
use super::*;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tine_store::cost_counters::{self, Counts};

fn measure(blocks: usize, referrers: usize) -> (Counts, Duration) {
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
    // Interval markers reused from the rename-reads instrumentation diff.
    let label = format!("{blocks}b-{referrers}r-general");
    fs::write(
        temp.path().join(format!("rr-BEGIN-{label}")),
        root.to_str().unwrap(),
    )
    .unwrap();
    let started = Instant::now();
    let report = rename_or_merge_page(&store, &host, "Target", "Renamed", None, None, &[]).unwrap();
    let call = started.elapsed();
    assert!(host.quiesce_for_tests(Duration::from_secs(60)));
    let counts = cost_counters::snapshot();
    fs::write(temp.path().join("rr-END"), b"").unwrap();

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
#[ignore = "isolated R5 measurement: run this exact test alone with --ignored --nocapture"]
fn single_vs_multi_cost_gate() {
    let samples = std::env::var("RR_SAMPLES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(7);
    println!(
        "R5 platform={} samples={samples} boundary=call-through-cleanup latency=call-median",
        std::env::consts::OS
    );
    println!("R5 blocks refs path files bytes fsyncs median_call_us raw_call_us");
    for blocks in [1, 60] {
        for referrers in [0, 1, 10] {
            let runs: Vec<_> = (0..samples).map(|_| measure(blocks, referrers)).collect();
            let counts = runs[0].0;
            let raw: Vec<_> = runs.iter().map(|(_, call)| call.as_micros()).collect();
            let mut times = raw.clone();
            times.sort_unstable();
            println!(
                "R5 {blocks} {referrers} general {} {} {} {} {raw:?}",
                counts.files_written,
                counts.bytes_written,
                counts.fsyncs,
                times[samples / 2]
            );
            for (other, _) in &runs {
                assert_eq!(other.files_written, counts.files_written);
                assert_eq!(other.bytes_written, counts.bytes_written);
                assert_eq!(other.fsyncs, counts.fsyncs);
            }
            #[cfg(target_os = "linux")]
            {
                // Recorded rename-reads special AND multi baselines, 2026-10-31.
                let bytes = match (blocks, referrers) {
                    (1, 0) => 227,
                    (1, 1) => 311,
                    (1, 10) => 1067,
                    (60, 0) => 2530,
                    (60, 1) => 6097,
                    (60, 10) => 38200,
                    _ => unreachable!(),
                };
                assert!(counts.files_written <= referrers as u64 + 3, "files rose");
                assert!(counts.bytes_written <= bytes, "bytes rose");
                assert!(counts.fsyncs <= 16 + 2 * referrers as u64, "fsyncs rose");
            }
        }
    }
}
