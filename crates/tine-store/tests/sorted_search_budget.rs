//! L7s3-02 / PC-3: a sorted friendly search with a small limit, over a
//! graph of a realistic size, answers inside a wall-clock budget.
//!
//! The retention counters in `query_plan/sort_tests.rs` pin what the search
//! keeps (at most `limit` candidates). This holds the other half of the cost
//! claim: that the time of the whole search, with a very common word that
//! matches thousands of blocks, stays small. It is a coarse guard, the kind
//! that catches a return to quadratic work or to a per-candidate copy of the
//! ancestor path, not a benchmark: the margin below is wide on purpose, so a
//! busy host does not make it flaky.
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{Duration, Instant};
use tine_core::query::ir::{Field, SortDir, ViewSettings};
use tine_store::{Cancel, SearchRequest, Store};

/// Pages and the blocks each holds. 2,000 pages of 12 blocks, 8 of them
/// matching, is 24,000 blocks and 16,000 matches: the density of a real graph
/// searched for a common word, at a fifth of the 10k-page generator's pages so
/// the fixture stays quick to write.
const PAGES: usize = 2_000;
const MATCHING: usize = 8;
const OTHER: usize = 4;
const LIMIT: usize = 20;
/// The budget for one warm sorted search, in milliseconds: about 5x the
/// measured time (see the note above the test).
const BUDGET_MS: u64 = 250;

fn write_graph(root: &std::path::Path) {
    std::fs::create_dir(root.join("pages")).unwrap();
    let mut seed = 0x9e37_79b9_u32;
    for page in 0..PAGES {
        let mut text = String::new();
        for block in 0..MATCHING {
            // A pseudo-random rank so the arrival order is neither sorted nor
            // reversed; the best twenty are scattered across the whole graph.
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            text.push_str(&format!(
                "- TODO budgetneedle task {page}-{block}\n  rank:: r{:07}\n",
                seed >> 4
            ));
        }
        for block in 0..OTHER {
            text.push_str(&format!("- unrelated note {page}-{block}\n"));
        }
        std::fs::write(root.join(format!("pages/Page{page:05}.md")), text).unwrap();
    }
}

fn sorted_request() -> SearchRequest {
    SearchRequest {
        transliteration: false,
        text: "budgetneedle".into(),
        within: None,
        page_limit: 0,
        block_limit: LIMIT,
        explain: false,
        page_match_scope: None,
        page_view: None,
        block_view: Some(ViewSettings {
            sort: vec![(Field("rank".into()), SortDir::Asc)],
            ..Default::default()
        }),
    }
}

/// The median of an odd number of samples.
fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

// Measured on this host (debug build, the test profile, `tine-heavy` slot, 16,000
// matches, limit 20), four runs of the code as it is: the first search 87-96 ms,
// the warm median 50-51 ms, so the 250 ms budget is a 4.9x margin over the warm
// median. The same test against the sorted-search files from before the top-K
// change (query_plan.rs, blocks.rs, pages.rs at 9fe3bcbbf) ran 133 ms first and 81 ms
// warm and passed too: this is a coarse guard against a return to something
// several times slower, not a proof of the top-K change (that is held by the
// retention counters in `query_plan/sort_tests.rs`). A tighter budget would
// start to fail on a busy host, and the lock below does not keep other
// processes off the CPU. The first search also builds the per-page property
// facts the sort key reads, so it is not the budgeted one.
#[test]
fn a_sorted_friendly_search_with_a_small_limit_stays_inside_its_budget() {
    let _integration_guard = crate::exclusive_integration_lock();
    let dir = tempfile::tempdir().unwrap();
    write_graph(dir.path());
    let (store, _, _) = Store::open(dir.path(), Default::default()).unwrap();
    let graph = store.whole_graph().unwrap();
    let request = sorted_request();
    let cancel = Cancel(Arc::new(AtomicBool::new(false)));

    let first_started = Instant::now();
    let first = graph.search(&request, &cancel).unwrap();
    let first_took = first_started.elapsed();
    assert_eq!(first.hits.len(), LIMIT, "the limit's worth of best blocks");
    assert!(
        first.has_more.blocks,
        "{} matches exceed {LIMIT}",
        PAGES * MATCHING
    );

    let mut warm = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let answer = graph.search(&request, &cancel).unwrap();
        warm.push(started.elapsed());
        assert_eq!(answer.hits.len(), LIMIT);
    }
    let warm = median(warm);
    eprintln!(
        "sorted_search_budget: {} matches, limit {LIMIT}: first {first_took:?}, warm median {warm:?}, budget {BUDGET_MS} ms",
        PAGES * MATCHING
    );
    assert!(
        warm < Duration::from_millis(BUDGET_MS),
        "a sorted search for {LIMIT} of {} matches took {warm:?} warm (budget {BUDGET_MS} ms)",
        PAGES * MATCHING
    );
}
