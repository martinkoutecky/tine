//! GH #623 (Ellis build 4): the asset full walk (store open and every focus
//! rescan's `refresh(Depth::Stamps)`) holds the store's one writer lock, which a page
//! click also takes. Stamping each asset by PATH is a per-file open on
//! Windows (250x under Defender on an unseen file), so a 12k-asset graph made
//! every focus return a multi-second critical section. The walk now stamps
//! from the directory listing, as the page walk does (spec §5.1).
//!
//! Counted, not timed (Linux has no per-open cost): one test per process, the
//! counters are process-global.
use std::fs;
use tine_store::{cost_counters, OpenOptions, Store, WatchMode};

/// By-path asset stamps at open and in two unchanged rescans of a graph with
/// `assets` asset files (config.edn and similar fixed files stamp by path too,
/// so the caller subtracts the zero-asset graph).
fn by_path_asset_stamps(assets: usize) -> (u64, u64) {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq", "assets/sub"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let write = |path: &std::path::Path, bytes: &[u8]| {
        fs::write(path, bytes).unwrap();
        // Count the walk, not legitimate rechecks inside the 2 s racy window (§5.4).
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    };
    write(&root.path().join("logseq/config.edn"), b"{}\n");
    write(&root.path().join("pages/A.md"), b"- a\n");
    for index in 0..assets {
        let dir = if index % 2 == 0 {
            "assets"
        } else {
            "assets/sub"
        };
        let path = root.path().join(format!("{dir}/f{index}.png"));
        write(&path, b"x");
    }
    cost_counters::reset();
    let (store, _, _) = Store::open(
        root.path(),
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap();
    drop(store.whole_graph().unwrap());
    let at_open = cost_counters::snapshot().asset_stamps_by_path;
    cost_counters::reset();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let in_rescans = cost_counters::snapshot().asset_stamps_by_path;
    store.close();
    (at_open, in_rescans)
}

#[test]
fn an_asset_full_walk_stamps_from_the_listing_and_opens_no_asset() {
    let _integration_guard = crate::exclusive_integration_lock();
    const ASSETS: usize = 60;
    let fixed = by_path_asset_stamps(0);
    let with_assets = by_path_asset_stamps(ASSETS);
    let extra = (
        with_assets.0.saturating_sub(fixed.0),
        with_assets.1.saturating_sub(fixed.1),
    );
    // Fixed config/CSS probes subtract exactly; watcher cycles are excluded.
    assert_eq!(
        extra,
        (0, 0),
        "GH #623: the asset baseline at open and each focus rescan's full asset walk must stamp \
         from the directory listing (`DirEntry::metadata`, no file open on Windows), never by \
         path; {ASSETS} assets added {} by-path stamps at open and {} in two rescans \
         (zero-asset graph: {fixed:?}); exemplar \
         crates/tine-store/src/model/page_identity.rs launch_listing_walk",
        extra.0,
        extra.1
    );
}
