//! Explicit store benchmark and unit-cost evidence for the OG-B-STORE lane.
//! Run with `--ignored --nocapture --test-threads=1`; no release build needed.
use std::{fs, time::Instant};
use tine_store::{
    cost_counters, Cancel, EditKind, OpenOptions, PageId, SaveBase, SaveOutcome, SearchRequest,
    Store,
};

#[test]
#[ignore = "explicit 2k/10k open/save benchmark"]
fn open_and_retained_snapshot_save_cost() {
    for pages in [2000, 10000] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::create_dir(dir.path().join("journals")).unwrap();
        for n in 0..pages {
            fs::write(
                dir.path().join(format!("pages/Page{n:05}.md")),
                "- unchanged\n",
            )
            .unwrap();
        }
        for blocks in [1, 60] {
            fs::write(
                dir.path().join(format!("pages/Target{blocks}.md")),
                "- before\n".repeat(blocks),
            )
            .unwrap();
        }
        let runs: usize = std::env::var("OG_B_COST_RUNS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);
        for run in 0..runs {
            let started = Instant::now();
            let store = Store::open(
                dir.path(),
                OpenOptions {
                    watch: tine_store::WatchMode::Poll,
                    ..Default::default()
                },
            )
            .unwrap()
            .0;
            let open_ms = started.elapsed().as_secs_f64() * 1000.0;
            let held = store.whole_graph().unwrap();
            let ready_ms = started.elapsed().as_secs_f64() * 1000.0;
            eprintln!(
                "BENCH pages={pages} run={run} open_ms={open_ms:.3} whole_graph_ms={ready_ms:.3}"
            );
            let request = SearchRequest {
                text: "unchanged".into(),
                within: None,
                page_limit: 20,
                block_limit: 20,
                explain: false,
                page_match_scope: None,
                page_view: None,
                block_view: None,
            };
            for sample in 0..5 {
                let started = Instant::now();
                let answer = held
                    .search(
                        &request,
                        &Cancel(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                            false,
                        ))),
                    )
                    .unwrap();
                std::hint::black_box(answer);
                let search_ms = started.elapsed().as_secs_f64() * 1000.0;
                let started = Instant::now();
                for _ in 0..100 {
                    std::hint::black_box(held.backlinks("Target1").unwrap());
                }
                let backlinks_ms = started.elapsed().as_secs_f64() * 10.0;
                let started = Instant::now();
                for _ in 0..100 {
                    std::hint::black_box(store.page(&PageId::from("pages/Target60.md")).unwrap());
                }
                let read_ms = started.elapsed().as_secs_f64() * 10.0;
                eprintln!("NEIGHBOR pages={pages} run={run} sample={sample} search_ms={search_ms:.3} backlinks_ms={backlinks_ms:.6} read_ms={read_ms:.6}");
            }
            for blocks in [1, 60] {
                let id = PageId::from(format!("pages/Target{blocks}.md"));
                for edit in 0..5 {
                    let read = store.page(&id).unwrap();
                    let mut doc = read.doc;
                    doc.blocks[0].raw = format!("after-{run}-{edit}");
                    cost_counters::reset();
                    let started = Instant::now();
                    let outcome = store.save(
                        EditKind::ReplacePage,
                        &id,
                        SaveBase::Existing(read.rev),
                        &doc,
                    );
                    let save_ms = started.elapsed().as_secs_f64() * 1000.0;
                    let cost = cost_counters::snapshot();
                    assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
                    assert_eq!(cost.files_written, 1);
                    let disk = fs::read(dir.path().join(id.as_str())).unwrap();
                    assert_eq!(cost.bytes_written, disk.len() as u64);
                    eprintln!("BENCH pages={pages} blocks={blocks} run={run} edit={edit} save_ms={save_ms:.3} bytes_written={} files_written={} instrumented_parses={} cache_copies={}", cost.bytes_written, cost.files_written, cost.parses, cost.cache_page_copies);
                    eprintln!(
                        "DETAIL pages={pages} blocks={blocks} run={run} edit={edit} {cost:?}"
                    );
                }
            }
            assert_eq!(
                held.corpus().pages.len(),
                pages + 2,
                "a retained view remains usable after edits"
            );
            drop(held);
            store.close();
        }
    }
}
