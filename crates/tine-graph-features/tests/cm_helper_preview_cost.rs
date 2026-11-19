//! KC2SE (merge reviews 2171d5664 / d4f72086d follow-up 1; PC-3, PC-8): the
//! rename helpers' question (`pages::rename_offer`: the alias dialog, the tag
//! union, the day move) on large synthetic pages. Contract
//! (rename-merge-domain.md "Rename helpers", "Day move"): one graph-wide stamp
//! walk per dialog (each file's stamp once: the one graph-wide cost, PC-1),
//! the named files read once each, the referrers found through the
//! reference index (not read), nothing written. Pinned as
//! counters at two sizes, so a read that grows with the page's blocks or with
//! the referrers fails here; the wall time is measured by the ignored
//! `helper_preview_timings` (release, median of 3), not asserted.
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tine_graph_features::pages::{rename_offer, RenameOffer};
use tine_store::cost_counters::{self, Counts};
use tine_store::Store;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Case {
    /// "ML" is only Machine Learning's alias: the alias dialog.
    Alias,
    /// Both pages have `tags::`: the merge offer with its union.
    TagMerge,
    /// A day renamed to a free name: the day move.
    DayMove,
}

/// A graph with the case's source page of `blocks` blocks and `referrers`
/// pages linking the name the question lists (plus 200 unrelated pages).
fn graph(case: Case, blocks: usize, referrers: usize) -> (tempfile::TempDir, PathBuf, Arc<Store>) {
    let temp = tempfile::Builder::new()
        .prefix("tine-kc2se-preview-")
        .tempdir()
        .unwrap();
    let root = temp.path().to_path_buf();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let body: String = (0..blocks)
        .map(|i| format!("- note {i} with some text\n"))
        .collect();
    let (source, linked) = match case {
        Case::Alias => {
            fs::write(
                root.join("pages/Machine Learning.md"),
                "alias:: AI, ML\n\n- ml\n",
            )
            .unwrap();
            fs::write(root.join("pages/ML notes.md"), &body).unwrap();
            ("pages/ML notes.md", "ML")
        }
        Case::TagMerge => {
            fs::write(root.join("pages/New.md"), "tags:: b, c\n\n- kept\n").unwrap();
            fs::write(root.join("pages/Old.md"), format!("tags:: a, b\n\n{body}")).unwrap();
            ("pages/Old.md", "Old")
        }
        Case::DayMove => {
            fs::write(
                root.join("journals/2026_11_18.md"),
                format!("alias:: Launch day\n\n{body}"),
            )
            .unwrap();
            ("journals/2026_11_18.md", "Nov 18th, 2026")
        }
    };
    assert!(root.join(source).exists());
    for i in 0..referrers {
        fs::write(
            root.join(format!("pages/Ref{i:05}.md")),
            format!("- see [[{linked}]] {i}\n"),
        )
        .unwrap();
    }
    for i in 0..200 {
        fs::write(
            root.join(format!("pages/Other{i:05}.md")),
            format!("- unrelated {i}\n"),
        )
        .unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (temp, root, store)
}

fn ask(case: Case, store: &Store) -> RenameOffer {
    let (old, new) = match case {
        Case::Alias => ("ML notes", "ML"),
        Case::TagMerge => ("Old", "New"),
        Case::DayMove => ("Nov 18th, 2026", "Trip"),
    };
    rename_offer(store, old, new, None).unwrap()
}

/// The question's counters and wall time on a fresh graph.
fn measure(case: Case, blocks: usize, referrers: usize) -> (Counts, Duration) {
    let (_temp, _root, store) = graph(case, blocks, referrers);
    cost_counters::reset();
    let started = Instant::now();
    let offer = ask(case, &store);
    let elapsed = started.elapsed();
    let counts = cost_counters::snapshot();
    match (case, &offer) {
        (Case::Alias, RenameOffer::Alias(dialog)) => assert!(dialog.own.is_some()),
        (Case::TagMerge, RenameOffer::Merge { tags, .. }) => assert!(tags.is_some()),
        (Case::DayMove, RenameOffer::DayMove(_)) => {}
        _ => panic!("{case:?}: unexpected offer {offer:?}"),
    }
    store.close();
    (counts, elapsed)
}

#[test]
fn the_helper_question_reads_the_named_files_once_and_writes_nothing() {
    let _guard = crate::exclusive_integration_lock();
    for case in [Case::Alias, Case::TagMerge, Case::DayMove] {
        let (small, _) = measure(case, 50, 20);
        let (large, _) = measure(case, 5_000, 2_000);
        eprintln!("KC2SE preview {case:?}\n 50 blocks / 20 referrers: {small:?}\n 5000 blocks / 2000 referrers: {large:?}");
        for counts in [&small, &large] {
            assert_eq!(
                counts.files_written, 0,
                "{case:?}: the question writes nothing"
            );
            assert_eq!(counts.fsyncs, 0, "{case:?}: the question syncs nothing");
        }
        // Neither the page's blocks nor the referrers add a file read, a
        // parse or a directory listing: the referrers come from the index.
        assert_eq!(
            large.full_reads, small.full_reads,
            "{case:?}: whole-file reads"
        );
        assert_eq!(large.parses, small.parses, "{case:?}: parses");
        assert_eq!(large.readdir, small.readdir, "{case:?}: directory listings");
        assert_eq!(
            large.store_reads, small.store_reads,
            "{case:?}: store reads"
        );
        assert!(
            large.store_reads <= 4,
            "{case:?}: the named files only: {large:?}"
        );
        // One graph-wide stamp walk per question: each file's stamp once
        // (the source and its partner, the referrers, 200 unrelated pages).
        let named = if case == Case::DayMove { 1 } else { 2 };
        for (counts, referrers) in [(&small, 20), (&large, 2_000)] {
            assert_eq!(
                counts.stamps_by_path,
                named + referrers + 200,
                "{case:?}: one stamp walk at {referrers} referrers"
            );
        }
    }
}

/// Wall time, release build, median of 3 (run with `--ignored --nocapture`
/// under `cargo test --release`); prints the load average beside it.
#[test]
#[ignore = "timing measurement, not a gate"]
fn helper_preview_timings() {
    let _guard = crate::exclusive_integration_lock();
    let load = fs::read_to_string("/proc/loadavg").unwrap_or_default();
    for case in [Case::Alias, Case::TagMerge, Case::DayMove] {
        for (blocks, referrers) in [(50, 20), (5_000, 2_000)] {
            let mut times: Vec<Duration> =
                (0..3).map(|_| measure(case, blocks, referrers).1).collect();
            times.sort();
            eprintln!("KC2SE timing {case:?} {blocks} blocks / {referrers} referrers: median {:?} (all {times:?}); load {}", times[1], load.trim());
        }
    }
}
