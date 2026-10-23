//! Audit 2026-10-22 L8-03 regression: a duplicate
//! journal day's stray holds the only reference to a top-level asset. A cold
//! open sets the stray aside without parsing it; the orphan inventory and the
//! orphan-trash guard must still count the stray's reference (contract: an
//! incomplete graph view cannot prove orphan-ness). In-scope threat: sync
//! delivery or an external editor writing the other journal file-name format.
use std::fs;
use std::time::{Duration, Instant};
use tine_graph_features::assets::{self, TrashOutcome};
use tine_store::{ChangeKind, OpenOptions, Store, Subscription, WatchMode};

const KEEPER: &str = "journals/2026_10_02.md";
const STRAY: &str = "journals/2026-10-02.md";

fn cold(files: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let files: Vec<_> = files
        .iter()
        .map(|(path, text)| (*path, text.as_bytes()))
        .collect();
    cold_bytes(&files)
}

fn cold_bytes(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq", "assets"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for (path, bytes) in files {
        fs::write(root.path().join(path), bytes).unwrap();
    }
    let store = Store::open(
        root.path(),
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    store.whole_graph_reconciled().unwrap();
    (root, store)
}

fn case(label: &str, keeper: &str, stray: &str) {
    let (root, store) = cold(&[
        (KEEPER, keeper),
        (STRAY, stray),
        ("assets/photo.png", "png-bytes"),
    ]);
    assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), 1);
    let listed: Vec<String> = assets::orphan_assets(&store)
        .unwrap()
        .into_iter()
        .map(|a| a.name)
        .collect();
    let outcome = assets::trash_asset(&store, "photo.png");
    let still_there = root.path().join("assets/photo.png").exists();
    eprintln!(
        "L8-03 {label}: orphan_assets lists {listed:?}; trash_asset -> {outcome:?}; file still in assets/: {still_there}"
    );
    assert!(
        !listed.contains(&"photo.png".to_owned()),
        "{label}: the Settings 'unused assets' scan offers photo.png although {STRAY} references it"
    );
    assert!(
        matches!(outcome, Ok(TrashOutcome::Referenced)) || outcome.is_err(),
        "{label}: trash_asset authorized trashing an asset the stray references: {outcome:?}"
    );
    assert!(still_there, "{label}: referenced asset was moved to trash");
    assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), 1);
}

/// Control: the reference in the presentation (keeper) file is counted.
#[test]
fn l8_03_control_keeper_reference_keeps_the_asset() {
    let _guard = crate::integration_lock();
    case(
        "control (reference in keeper)",
        "- the day ![photo](../assets/photo.png)\n",
        "- stray, no media\n",
    );
}

#[test]
fn l8_03_stray_block_reference_keeps_the_asset_after_a_cold_open() {
    let _guard = crate::integration_lock();
    case(
        "stray block reference",
        "- the day\n",
        "- stray ![photo](../assets/photo.png)\n",
    );
}

#[test]
fn l8_03_stray_preamble_reference_keeps_the_asset_after_a_cold_open() {
    let _guard = crate::integration_lock();
    case(
        "stray preamble reference",
        "- the day\n",
        "cover:: ![photo](../assets/photo.png)\n\n- stray body\n",
    );
}

fn await_stray_change(changes: &Subscription, expected: ChangeKind) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(change) = changes.try_recv().unwrap() {
            if change
                .files
                .iter()
                .any(|(id, kind, _)| id.as_str() == STRAY && *kind == expected)
            {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "watcher did not publish {expected:?} for {STRAY}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn l8_03_watcher_arrival_protects_the_asset_and_removal_releases_it() {
    let _guard = crate::integration_lock();
    let (root, store) = cold(&[(KEEPER, "- the day\n"), ("assets/photo.png", "png")]);
    assert_eq!(assets::orphan_assets(&store).unwrap()[0].name, "photo.png");
    let changes = store.subscribe();
    // No explicit refresh: the poll watcher must deliver both transitions.
    fs::write(
        root.path().join(STRAY),
        "cover:: ![photo](../assets/photo.png)\n\n- stray\n",
    )
    .unwrap();
    await_stray_change(&changes, ChangeKind::Created);
    let before: Vec<_> = store
        .whole_graph()
        .unwrap()
        .corpus()
        .pages
        .into_iter()
        .map(|page| page.id)
        .collect();
    assert!(assets::orphan_assets(&store).unwrap().is_empty());
    assert_eq!(
        assets::trash_asset(&store, "photo.png").unwrap(),
        TrashOutcome::Referenced
    );
    // The existing live-arrival path may already parse a stray. Asset safety
    // must not change that membership or the file presented for this day.
    let after: Vec<_> = store
        .whole_graph()
        .unwrap()
        .corpus()
        .pages
        .into_iter()
        .map(|page| page.id)
        .collect();
    assert_eq!(after, before);
    assert_eq!(store.journal_id(tine_store::Day(20261002)).as_str(), KEEPER);
    assert_eq!(
        fs::read_to_string(root.path().join(KEEPER)).unwrap(),
        "- the day\n"
    );

    fs::remove_file(root.path().join(STRAY)).unwrap();
    await_stray_change(&changes, ChangeKind::Removed);
    assert_eq!(assets::orphan_assets(&store).unwrap()[0].name, "photo.png");
    assert_eq!(
        assets::trash_asset(&store, "photo.png").unwrap(),
        TrashOutcome::Trashed
    );
    assert!(!root.path().join("assets/photo.png").exists());
}

#[test]
fn l8_03_trash_rechecks_stray_bytes_after_the_unused_listing() {
    let _guard = crate::integration_lock();
    let (root, store) = cold(&[
        (KEEPER, "- the day\n"),
        (STRAY, "- no reference\n"),
        ("assets/photo.png", "png"),
    ]);
    assert_eq!(assets::orphan_assets(&store).unwrap()[0].name, "photo.png");
    fs::write(
        root.path().join(STRAY),
        "- new ![photo](../assets/photo.png)\n",
    )
    .unwrap();
    assert_eq!(
        assets::trash_asset(&store, "photo.png").unwrap(),
        TrashOutcome::Referenced
    );
    assert!(root.path().join("assets/photo.png").exists());
}

#[test]
fn l8_03_unreadable_or_unanalyzable_cold_stray_refuses_listing_and_trash_by_name() {
    let _guard = crate::integration_lock();
    let deep: String = (0..130)
        .map(|depth| format!("{}- nested\n", "  ".repeat(depth)))
        .collect();
    for bytes in [b"\xff\xfe".as_slice(), deep.as_bytes()] {
        let (root, store) = cold_bytes(&[
            (KEEPER, b"- the day\n"),
            (STRAY, bytes),
            ("assets/photo.png", b"png"),
        ]);
        let listing_error = assets::orphan_assets(&store).unwrap_err();
        assert!(listing_error.to_string().contains(STRAY), "{listing_error}");
        let trash_error = assets::trash_asset(&store, "photo.png").unwrap_err();
        assert!(trash_error.to_string().contains(STRAY), "{trash_error}");
        assert!(root.path().join("assets/photo.png").exists());
    }
}

#[test]
fn l8_03_org_stray_uses_its_format_parser_for_preamble_and_block_references() {
    let _guard = crate::integration_lock();
    for text in [
        "#+COVER: [[../assets/photo.png]]\n\n* stray\n",
        "* stray [[../assets/photo.png]]\n",
    ] {
        let (root, store) = cold(&[
            (KEEPER, "- the day\n"),
            ("journals/2026-10-02.org", text),
            ("assets/photo.png", "png"),
        ]);
        assert!(assets::orphan_assets(&store).unwrap().is_empty());
        assert_eq!(
            assets::trash_asset(&store, "photo.png").unwrap(),
            TrashOutcome::Referenced
        );
        assert!(root.path().join("assets/photo.png").exists());
    }
}

#[test]
fn l8_03_inventory_only_reads_and_parses_set_aside_files() {
    let _guard = crate::integration_lock();
    let (_root, store) = cold(&[
        (KEEPER, "- the day\n"),
        ("pages/A.md", "- indexed\n"),
        ("pages/B.md", "- also indexed\n"),
    ]);
    tine_store::cost_counters::reset();
    store.referenced_assets().unwrap();
    store.referenced_assets().unwrap();
    let counts = tine_store::cost_counters::snapshot();
    assert_eq!(counts.full_reads, 0, "{counts:?}");
    assert_eq!(counts.parses, 0, "{counts:?}");

    let (_stray_root, stray_store) = cold(&[
        (KEEPER, "- the day\n"),
        (STRAY, "- ![photo](../assets/photo.png)\n"),
        ("pages/A.md", "- indexed\n"),
        ("pages/B.md", "- also indexed\n"),
    ]);
    tine_store::cost_counters::reset();
    assert!(stray_store
        .referenced_assets()
        .unwrap()
        .contains("photo.png"));
    assert!(stray_store
        .referenced_assets()
        .unwrap()
        .contains("photo.png"));
    let counts = tine_store::cost_counters::snapshot();
    assert_eq!(counts.full_reads, 2, "{counts:?}");
    assert_eq!(counts.parses, 2, "{counts:?}");
    assert_eq!(counts.files_written, 0, "{counts:?}");
    assert_eq!(counts.fsyncs, 0, "{counts:?}");
}
