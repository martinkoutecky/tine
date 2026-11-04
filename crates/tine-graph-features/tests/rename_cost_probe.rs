//! Rename unit-cost probe (I-25, fam 16). Ignored: run with
//! `TINE_RENAME_PROBE_ROOT=<copy of a generated 10k graph> TINE_RENAME_PROBE_OLD=..`
//! `cargo test -p tine-graph-features --release --test integration rename_cost_probe:: -- --ignored --nocapture`.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tine_graph_features::pages;
use tine_store::Store;

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for dir in ["pages", "journals"] {
        for entry in fs::read_dir(root.join(dir)).unwrap().flatten() {
            let name = format!("{dir}/{}", entry.file_name().to_string_lossy());
            out.insert(name, fs::read(entry.path()).unwrap());
        }
    }
    out
}

/// Bytes the calling thread read through read(2) so far (`rchar`, Linux
/// only). The rename runs on this thread; background threads are excluded.
fn bytes_read() -> u64 {
    fs::read_to_string("/proc/thread-self/io")
        .ok()
        .and_then(|io| {
            io.lines()
                .find_map(|line| line.strip_prefix("rchar: ")?.trim().parse().ok())
        })
        .unwrap_or(0)
}

#[test]
#[ignore]
fn rename_cost_at_scale() {
    let _integration_guard = crate::integration_lock();
    let root = std::path::PathBuf::from(std::env::var("TINE_RENAME_PROBE_ROOT").unwrap());
    let old = std::env::var("TINE_RENAME_PROBE_OLD").unwrap();
    let new = format!("{old} renamed");
    let before = snapshot(&root);
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let warm = Instant::now();
    store.whole_graph().unwrap();
    let warm = warm.elapsed();
    let read_before = bytes_read();
    tine_store::cost_counters::reset();
    let started = Instant::now();
    hosted(&store, |host| {
        pages::rename_page_expected(&store, host, &old, &new, None)
    })
    .unwrap();
    let rename = started.elapsed();
    let counts = tine_store::cost_counters::snapshot();
    let read = bytes_read() - read_before;
    let after = snapshot(&root);
    let written: Vec<_> = after
        .iter()
        .filter(|(path, bytes)| before.get(*path) != Some(*bytes))
        .collect();
    let bytes: usize = written.iter().map(|(_, bytes)| bytes.len()).sum();
    println!(
        "PROBE old={old:?} warm_ms={} rename_ms={} files_written={} bytes_written={bytes} \
         dirs_listed={} full_reads={} bytes_read={read}",
        warm.as_millis(),
        rename.as_millis(),
        written.len(),
        counts.readdir,
        counts.full_reads,
    );
}

fn median(mut samples: Vec<u128>) -> u128 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// og-perf-rename-walks: what a rename and a delete pay that grows with the
/// graph. Ignored: `TINE_RENAME_PROBE_ROOT=<copy of a generated 10k graph>`
/// `TINE_RENAME_PROBE_OLD=<a page name>` (`TINE_RENAME_PROBE_ROUNDS`, default
/// 7), release build, run alone. Prints the median wall time of one
/// `refresh(Depth::Stamps)`, of the export-definition query a rename plans
/// with, of a rename, and of a page delete, with the files each operation
/// stamped from a full walk's listing (graph text, assets).
#[test]
#[ignore]
fn rename_and_delete_walks_at_scale() {
    use tine_core::query::ir::{ExecutionContext, ViewSettings};
    let _integration_guard = crate::integration_lock();
    let root = std::path::PathBuf::from(std::env::var("TINE_RENAME_PROBE_ROOT").unwrap());
    let old = std::env::var("TINE_RENAME_PROBE_OLD").unwrap();
    let rounds: usize = std::env::var("TINE_RENAME_PROBE_ROUNDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(7);
    let new = format!("{old} renamed");
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let app = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app.path()).unwrap();
    let settle = || assert!(host.quiesce_for_tests(std::time::Duration::from_secs(60)));
    settle();
    let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
        .as_u64()
        .unwrap();
    let definitions = tine_core::query::wire_parse::parse_query_pair(
        &tine_core::export_definition::definitions_query(),
        tine_core::query::wire_parse::QueryTextDialect::Og,
        &[],
        tine_core::query::registry::Registry::none(),
    );
    let (mut refresh, mut query, mut rename, mut delete) = (vec![], vec![], vec![], vec![]);
    let (mut rename_walked, mut delete_walked) = (None, None);
    for round in 0..rounds {
        let started = Instant::now();
        store.refresh(tine_store::Depth::Stamps).unwrap();
        refresh.push(started.elapsed().as_micros());

        tine_store::cost_counters::reset();
        let started = Instant::now();
        pages::rename_or_merge_page(&store, &host, &old, &new, None, None, &[]).unwrap();
        rename.push(started.elapsed().as_micros());
        let counts = tine_store::cost_counters::snapshot();
        rename_walked = Some((counts.listing_stamps, counts.asset_listing_stamps));
        settle();
        pages::rename_or_merge_page(&store, &host, &new, &old, None, None, &[]).unwrap();
        settle();
        // The query on a generation it has not run on yet (memoized per
        // generation; the rename back moved it), as the rename's own plan
        // meets it after an edit.
        let graph = store.whole_graph().unwrap();
        let started = Instant::now();
        graph
            .query_ir(
                tine_store::IrRequest::Run {
                    query: &definitions.query,
                    view: &ViewSettings::default(),
                    context: &ExecutionContext::default(),
                },
                &tine_store::Cancel::default(),
            )
            .unwrap();
        query.push(started.elapsed().as_micros());

        let name = format!("Probe delete {round}");
        let rel = format!("pages/{name}.md");
        fs::write(root.join(&rel), "- delete me\n").unwrap();
        store.refresh(tine_store::Depth::Stamps).unwrap();
        settle();
        tine_store::cost_counters::reset();
        let started = Instant::now();
        let deleted = pages::delete_page_expected(
            &store,
            &host,
            session,
            &name,
            tine_core::model::PageKind::Page,
            Some(&rel),
        )
        .unwrap();
        delete.push(started.elapsed().as_micros());
        let counts = tine_store::cost_counters::snapshot();
        delete_walked = Some((counts.listing_stamps, counts.asset_listing_stamps));
        assert_ne!(deleted, tine_store::PageOperation::Refused);
        settle();
        assert!(!root.join(&rel).exists());
    }
    println!(
        "WALKS old={old:?} rounds={rounds} refresh_us={} definitions_query_us={} rename_us={} \
         delete_us={} rename_walked(text,assets)={:?} delete_walked(text,assets)={:?}",
        median(refresh),
        median(query),
        median(rename),
        median(delete),
        rename_walked.unwrap(),
        delete_walked.unwrap(),
    );
    drop(host);
    store.close();
}

/// CM step 2, batch 8: what the reference build costs at scale. Ignored:
/// `TINE_RENAME_PROBE_ROOT=<fresh copy of a generated 10k graph>`
/// (`TINE_RENAME_PROBE_ROUNDS`, default 7), release build, run alone. Prints
/// the wall time of a cold open plus the whole-graph build (which primes
/// every page's reference projections) and the median of All pages'
/// `page_backlink_counts`, one forward walk of every referring block.
#[test]
#[ignore]
fn reference_build_at_scale() {
    let _integration_guard = crate::integration_lock();
    let root = std::path::PathBuf::from(std::env::var("TINE_RENAME_PROBE_ROOT").unwrap());
    let rounds: usize = std::env::var("TINE_RENAME_PROBE_ROUNDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(7);
    let started = Instant::now();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    let open = started.elapsed().as_micros();
    let mut counts = vec![];
    let mut rows = 0;
    for _ in 0..rounds {
        let started = Instant::now();
        let (files, references) = graph.page_backlink_counts();
        counts.push(started.elapsed().as_micros());
        rows = files.iter().map(|(_, n)| n).sum::<usize>()
            + references.iter().map(|(_, n)| n).sum::<usize>();
    }
    println!(
        "REFBUILD rounds={rounds} open_whole_graph_us={open} backlink_counts_us={} counted_rows={rows}",
        median(counts),
    );
    drop(graph);
    store.close();
}
