//! Concord base ledger (og ADR 0056): retention, verification, pins, prune,
//! the change-feed wiring through `watcher::concord_observe`, and the two
//! failure contracts — an unusable ledger never blocks a save or a resolve,
//! and a stale or foreign base never produces silent loss.

use super::*;
use crate::state::GraphSlot;
use std::collections::HashMap;
use std::sync::Arc;
use tine_graph_features::conflicts;
use tine_store::{OpenOptions, PageId, SaveBase, Store, WatchMode};

const COPY: &str = "pages/Desk.sync-conflict-20260929-101010-ABCDEFG.md";
const ID: &str = "aaaaaaaa-0000-0000-0000-0000000000d5";

/// The fixture page: a shared intro and one identified block reading `text`.
fn body(text: &str) -> String {
    format!("- shared intro line\n- {text}\n  id:: {ID}\n")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tine-concord-ledger-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("graph/pages")).unwrap();
    std::fs::create_dir_all(dir.join("graph/journals")).unwrap();
    dir
}

/// A bound slot with its ledger attached under `<dir>/appdata` (or under the
/// given app-data path), plus a subscription standing in for the dispatch
/// thread.
fn open_slot(dir: &Path, app_data: PathBuf) -> (Arc<GraphSlot>, tine_store::Subscription) {
    let root = dir.join("graph");
    let store = Store::open(
        &root,
        OpenOptions {
            approved_external_assets: None,
            watch: WatchMode::Poll,
        },
    )
    .unwrap()
    .0;
    store.whole_graph().unwrap();
    let subscription = store.subscribe();
    let slot = Arc::new(GraphSlot::new(store, root));
    attach(Some(app_data), &slot);
    if let Some(ledger) = slot.concord_ledger.get() {
        assert!(ledger.drain_for_exit(Instant::now() + Duration::from_secs(10)));
    }
    (slot, subscription)
}

/// Deliver every published change the way the dispatch thread does, then
/// wait for the ledger worker. Returns whether the conflict queue changed.
fn pump(slot: &GraphSlot, subscription: &tine_store::Subscription) -> bool {
    let mut changed = false;
    while let Some(change) = subscription.try_recv().unwrap() {
        changed |= crate::watcher::concord_observe(slot, &change);
    }
    assert!(slot.concord_ledger.get().map_or(true, |l| l
        .drain_for_exit(Instant::now() + Duration::from_secs(10))));
    changed
}

/// An own (Tine) save setting the identified block of page `rel` to `text`.
fn save(slot: &GraphSlot, rel: &str, text: &str) {
    let id = PageId::from(rel);
    let read = slot.store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[1].raw = format!("{text}\nid:: {ID}");
    let mut tx = slot
        .store
        .transaction(Some(tine_store::EditKind::ReplacePage));
    tx.save_page(
        &[tine_store::EditKind::ReplacePage],
        &id,
        SaveBase::Existing(read.rev),
        &doc,
    );
    assert!(matches!(
        tx.commit(),
        tine_store::TxOutcome::Committed { .. }
    ));
    assert_eq!(
        std::fs::read_to_string(slot.root_key.join(rel)).unwrap(),
        body(text)
    );
}

/// The pre-selection a user would confirm: each row's suggestion, and for
/// rows without one, keep both sides.
fn preselected(diff: &tine_core::sync_diff::SyncConflictDiff) -> HashMap<String, String> {
    diff.rows
        .iter()
        .map(|row| {
            let choice = row.suggestion.clone().unwrap_or_else(|| {
                if matches!(row.kind, tine_core::sync_diff::RowKind::Unchanged) {
                    "mine".to_owned()
                } else {
                    "both".to_owned()
                }
            });
            (row.id.clone(), choice)
        })
        .collect()
}

fn external(slot: &GraphSlot, rel: &str, text: &str) {
    let path = slot.root_key.join(rel);
    let temp = path.with_extension("ext-tmp");
    std::fs::write(&temp, text).unwrap();
    std::fs::rename(temp, path).unwrap();
    slot.store.scan_refresh().unwrap();
}

fn files(dir: &Path) -> LedgerFiles {
    LedgerFiles {
        dir: dir.to_path_buf(),
    }
}

fn file_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| {
                    if e.path().is_dir() {
                        file_count(&e.path())
                    } else {
                        1
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

#[test]
fn retention_keeps_the_last_two_distinct_texts_and_evicts_the_rest() {
    let dir = scratch("retention");
    let ledger = files(&dir.join("ledger"));
    for text in ["one", "two", "two", "three"] {
        ledger.record("pages/P.md", text.as_bytes()).unwrap();
    }
    assert_eq!(ledger.retained("pages/P.md"), vec!["three", "two"]);
    // Steady state: K blobs plus the index; the evicted blob is gone.
    assert_eq!(file_count(&ledger.page_dir("pages/P.md")), RETAINED + 1);
    // Re-recording an older retained text moves it to the front, no new blob.
    ledger.record("pages/P.md", b"two").unwrap();
    assert_eq!(ledger.retained("pages/P.md"), vec!["two", "three"]);
    // Re-recording the newest text writes nothing at all.
    let index = ledger.page_dir("pages/P.md").join("index.json");
    let before = std::fs::metadata(&index).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    ledger.record("pages/P.md", b"two").unwrap();
    assert_eq!(
        std::fs::metadata(&index).unwrap().modified().unwrap(),
        before
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn corrupt_missing_or_foreign_entries_answer_no_base() {
    let dir = scratch("corrupt");
    let ledger = files(&dir.join("ledger"));
    ledger.record("pages/A.md", b"alpha").unwrap();
    ledger.record("pages/A.md", b"alpha 2").unwrap();
    // A torn/bit-flipped blob is skipped; the other retained text survives.
    std::fs::write(
        ledger.page_dir("pages/A.md").join(sha(b"alpha 2")),
        "alpha X",
    )
    .unwrap();
    assert_eq!(ledger.retained("pages/A.md"), vec!["alpha"]);
    // An index from another page (e.g. restored/copied app data) is foreign.
    ledger.record("pages/B.md", b"beta").unwrap();
    std::fs::copy(
        ledger.page_dir("pages/A.md").join("index.json"),
        ledger.page_dir("pages/B.md").join("index.json"),
    )
    .unwrap();
    assert!(ledger.retained("pages/B.md").is_empty());
    // An unreadable index or an unknown schema answers nothing.
    std::fs::write(
        ledger.page_dir("pages/A.md").join("index.json"),
        "{not json",
    )
    .unwrap();
    assert!(ledger.retained("pages/A.md").is_empty());
    let other_schema = PageIndex {
        schema: LEDGER_SCHEMA + 1,
        path: "pages/A.md".into(),
        revs: vec![sha(b"alpha")],
    };
    std::fs::write(
        ledger.page_dir("pages/A.md").join("index.json"),
        serde_json::to_vec(&other_schema).unwrap(),
    )
    .unwrap();
    assert!(ledger.retained("pages/A.md").is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_pin_is_first_wins_skips_either_sides_current_text_and_drops() {
    let dir = scratch("pin");
    let ledger = files(&dir.join("ledger"));
    ledger.record("pages/Desk.md", b"- ancestor\n").unwrap();
    ledger.record("pages/Desk.md", b"- admitted\n").unwrap();
    // The winner's newest entry equals its current bytes (the admission
    // artifact): the pin takes the older ancestor.
    ledger
        .pin(
            COPY,
            "pages/Desk.md",
            [Some(sha(b"- admitted\n")), Some(sha(b"- copy\n"))],
        )
        .unwrap();
    assert_eq!(ledger.pinned(COPY).as_deref(), Some("- ancestor\n"));
    // First wins: later observations never move it.
    ledger.record("pages/Desk.md", b"- later\n").unwrap();
    ledger.pin(COPY, "pages/Desk.md", [None, None]).unwrap();
    assert_eq!(ledger.pinned(COPY).as_deref(), Some("- ancestor\n"));
    // Retention evicting the winner's ancestor does not evict the pin.
    assert!(!ledger
        .retained("pages/Desk.md")
        .contains(&"- ancestor\n".to_owned()));
    ledger.drop_pin(COPY).unwrap();
    assert_eq!(ledger.pinned(COPY), None);
    ledger.drop_pin(COPY).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn prune_drops_gone_pages_orphans_torn_temps_and_stale_pins() {
    let dir = scratch("prune");
    std::fs::write(dir.join("graph/pages/Kept.md"), "- kept\n").unwrap();
    let (slot, _sub) = open_slot(&dir, dir.join("appdata"));
    let ledger = slot.concord_ledger.get().unwrap();
    let on_disk = ledger.files();
    on_disk.record("pages/Kept.md", b"- kept\n").unwrap();
    on_disk.record("pages/Gone.md", b"- gone\n").unwrap();
    let kept = on_disk.page_dir("pages/Kept.md");
    std::fs::write(kept.join("orphan-blob"), "x").unwrap();
    std::fs::write(kept.join(".index.json.1.2.tmp"), "torn").unwrap();
    on_disk.record("pages/Desk.md", b"- base\n").unwrap();
    on_disk.pin(COPY, "pages/Desk.md", [None, None]).unwrap();
    assert!(on_disk.pinned(COPY).is_some());
    let removed = on_disk.prune(&slot.store).unwrap();
    assert!(removed >= 5, "removed {removed}");
    assert_eq!(on_disk.retained("pages/Kept.md"), vec!["- kept\n"]);
    assert_eq!(file_count(&kept), 2);
    assert!(!on_disk.page_dir("pages/Gone.md").exists());
    assert!(!on_disk.page_dir("pages/Desk.md").exists());
    assert_eq!(on_disk.pinned(COPY), None, "the copy is not in the graph");
    assert_eq!(file_count(&on_disk.dir.join("pins")), 0);
    drop(slot);
    let _ = std::fs::remove_dir_all(dir);
}

/// The Syncthing journey end to end: Tine saves the ancestor, then the local
/// edit; Syncthing delivers the other device's edit as a conflict copy. The
/// review is 3-way with a `merged` proposal, and resolving it writes the
/// composed body, trashes the copy and drops the pin.
#[test]
fn a_syncthing_copy_resolves_three_way_with_the_ledger_base() {
    let dir = scratch("e2e");
    std::fs::write(dir.join("graph/pages/Desk.md"), body("seed")).unwrap();
    let (slot, sub) = open_slot(&dir, dir.join("appdata"));
    slot.conflict_queue.inventory(&slot.store);
    save(&slot, "pages/Desk.md", "Desktop 5");
    pump(&slot, &sub);
    save(&slot, "pages/Desk.md", "Desktop");
    pump(&slot, &sub);
    external(&slot, COPY, &body("Desktop 5 kk"));
    assert!(pump(&slot, &sub), "the copy enters the queue");
    let before_winner = std::fs::read_to_string(dir.join("graph/pages/Desk.md")).unwrap();
    let before_copy = std::fs::read_to_string(dir.join("graph").join(COPY)).unwrap();
    assert_eq!(
        (before_winner, before_copy.clone()),
        (body("Desktop"), body("Desktop 5 kk"))
    );
    assert_eq!(
        slot.conflict_queue
            .inventory(&slot.store)
            .sync_conflicts
            .len(),
        1
    );

    let ledger = slot.concord_ledger.get().unwrap();
    let bases = ledger.conflict_bases(COPY, "pages/Desk.md");
    assert_eq!(bases.first(), Some(&body("Desktop 5")), "the pin");
    let diff = conflicts::sync_conflict_diff(&slot.store, "pages/Desk.md", COPY, &bases)
        .unwrap()
        .unwrap();
    assert!(diff.three_way);
    assert_eq!(
        diff.merge_base_rev.as_deref(),
        Some(sha(body("Desktop 5").as_bytes()).as_str())
    );
    let row = diff
        .rows
        .iter()
        .find(|r| r.merged.is_some())
        .expect("a merged proposal");
    assert_eq!(row.suggestion.as_deref(), Some("merged"));
    let decisions = preselected(&diff);
    conflicts::resolve_sync_conflict(
        &slot.store,
        "pages/Desk.md",
        COPY,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        diff.merge_base_rev.as_deref(),
        &ledger.conflict_bases(COPY, "pages/Desk.md"),
        "union",
    )
    .unwrap();
    let after = std::fs::read_to_string(dir.join("graph/pages/Desk.md")).unwrap();
    assert_eq!(after, body("Desktop kk"));
    assert!(!dir.join("graph").join(COPY).exists());
    assert!(pump(&slot, &sub), "the resolved copy leaves the queue");
    assert!(slot.conflict_queue.inventory(&slot.store).queue.is_empty());
    assert_eq!(ledger.files().pinned(COPY), None, "resolve dropped the pin");
    // The merged body is now the winner's newest agreed text.
    assert_eq!(
        ledger.files().retained("pages/Desk.md")[0],
        body("Desktop kk")
    );
    println!(
        "E2E before: winner {:?} copy {before_copy:?}; after: winner {after:?}, copy trashed",
        body("Desktop")
    );
    drop(slot);
    let _ = std::fs::remove_dir_all(dir);
}

/// Syncthing replaces the winner and drops the copy in one scan: the pin is
/// taken before the admission is recorded, so it is the ancestor, not the
/// delivered bytes.
#[test]
fn a_copy_arriving_with_the_winner_admission_pins_the_ancestor() {
    let dir = scratch("admission");
    std::fs::write(dir.join("graph/pages/Desk.md"), body("seed")).unwrap();
    let (slot, sub) = open_slot(&dir, dir.join("appdata"));
    save(&slot, "pages/Desk.md", "Desktop 5");
    pump(&slot, &sub);
    std::fs::write(dir.join("graph").join(COPY), body("Desktop")).unwrap();
    external(&slot, "pages/Desk.md", &body("Desktop 5 kk"));
    pump(&slot, &sub);
    let ledger = slot.concord_ledger.get().unwrap();
    assert_eq!(ledger.files().pinned(COPY), Some(body("Desktop 5")));
    assert_eq!(
        ledger.files().retained("pages/Desk.md")[0],
        body("Desktop 5 kk"),
        "the admission is recorded after the pin"
    );
    let diff = conflicts::sync_conflict_diff(
        &slot.store,
        "pages/Desk.md",
        COPY,
        &ledger.conflict_bases(COPY, "pages/Desk.md"),
    )
    .unwrap()
    .unwrap();
    assert!(diff.three_way);
    assert!(diff
        .rows
        .iter()
        .any(|r| r.suggestion.as_deref() == Some("merged")));
    drop(slot);
    let _ = std::fs::remove_dir_all(dir);
}

/// Contract 2: an unwritable ledger location (a file where the directory
/// should be) never blocks, delays or fails a save or a resolve; the review
/// degrades to the 2-way diff.
#[test]
fn an_unwritable_ledger_never_blocks_saves_or_resolves() {
    let dir = scratch("unwritable");
    std::fs::write(dir.join("graph/pages/Desk.md"), body("seed")).unwrap();
    std::fs::write(dir.join("appdata-file"), "not a directory").unwrap();
    let (slot, sub) = open_slot(&dir, dir.join("appdata-file"));
    save(&slot, "pages/Desk.md", "Desktop 5");
    pump(&slot, &sub);
    save(&slot, "pages/Desk.md", "Desktop");
    pump(&slot, &sub);
    external(&slot, COPY, &body("Desktop 5 kk"));
    pump(&slot, &sub);
    let ledger = slot.concord_ledger.get().unwrap();
    assert!(ledger.conflict_bases(COPY, "pages/Desk.md").is_empty());
    let diff = conflicts::sync_conflict_diff(&slot.store, "pages/Desk.md", COPY, &[])
        .unwrap()
        .unwrap();
    assert!(!diff.three_way && diff.merge_base_rev.is_none());
    let decisions: HashMap<_, _> = diff
        .rows
        .iter()
        .map(|row| (row.id.clone(), "theirs".to_owned()))
        .collect();
    conflicts::resolve_sync_conflict(
        &slot.store,
        "pages/Desk.md",
        COPY,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        None,
        &[],
        "union",
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("graph/pages/Desk.md")).unwrap(),
        body("Desktop 5 kk")
    );
    drop(slot);
    let _ = std::fs::remove_dir_all(dir);
}

/// Contract 3: a base that moved between review and apply refuses a
/// `"merged"` resolve and writes nothing (scenario: sync delivery or an
/// honest concurrent instance moved the ledger); a base from another page
/// yields no suggestion that silently discards either side.
#[test]
fn a_stale_or_foreign_base_never_loses_data_silently() {
    let dir = scratch("stale");
    std::fs::write(dir.join("graph/pages/Desk.md"), body("seed")).unwrap();
    let (slot, sub) = open_slot(&dir, dir.join("appdata"));
    save(&slot, "pages/Desk.md", "Desktop 5");
    pump(&slot, &sub);
    save(&slot, "pages/Desk.md", "Desktop");
    pump(&slot, &sub);
    external(&slot, COPY, &body("Desktop 5 kk"));
    pump(&slot, &sub);
    let ledger = slot.concord_ledger.get().unwrap();
    let diff = conflicts::sync_conflict_diff(
        &slot.store,
        "pages/Desk.md",
        COPY,
        &ledger.conflict_bases(COPY, "pages/Desk.md"),
    )
    .unwrap()
    .unwrap();
    let decisions = preselected(&diff);
    assert!(decisions.values().any(|d| d == "merged"));
    // The ledger moved: the base the review showed is no longer offered.
    let moved = vec![body("Desktop 4")];
    let err = conflicts::resolve_sync_conflict(
        &slot.store,
        "pages/Desk.md",
        COPY,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        diff.merge_base_rev.as_deref(),
        &moved,
        "union",
    )
    .unwrap_err();
    assert!(err.to_string().contains("merge base changed"), "{err}");
    // A failed ledger read at apply time refuses the merged row the same way.
    let err = conflicts::resolve_sync_conflict(
        &slot.store,
        "pages/Desk.md",
        COPY,
        &decisions,
        &diff.base_rev,
        &diff.conflict_rev,
        diff.merge_base_rev.as_deref(),
        &[],
        "union",
    )
    .unwrap_err();
    assert!(err.to_string().contains("merge base changed"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.join("graph/pages/Desk.md")).unwrap(),
        body("Desktop")
    );
    assert!(dir.join("graph").join(COPY).exists());
    // A base from a different page: confirming its pre-selection keeps every
    // block of both sides (a foreign base has no block to justify a drop).
    let foreign = vec!["- an unrelated page\n- with other blocks\n".to_owned()];
    let diff = conflicts::sync_conflict_diff(&slot.store, "pages/Desk.md", COPY, &foreign)
        .unwrap()
        .unwrap();
    assert!(diff.three_way);
    // Non-merged decisions never read the base: a ledger failure at apply
    // time does not refuse them.
    conflicts::resolve_sync_conflict(
        &slot.store,
        "pages/Desk.md",
        COPY,
        &preselected(&diff),
        &diff.base_rev,
        &diff.conflict_rev,
        diff.merge_base_rev.as_deref(),
        &[],
        "union",
    )
    .unwrap();
    let merged = std::fs::read_to_string(dir.join("graph/pages/Desk.md")).unwrap();
    for text in ["shared intro line", "- Desktop\n", "Desktop 5 kk"] {
        assert!(merged.contains(text), "{text:?} lost: {merged}");
    }
    drop(slot);
    let _ = std::fs::remove_dir_all(dir);
}

/// I-25 unit cost, measured: bytes and files one recorded save writes on a
/// 1-block and a 60-block page, and the steady-state footprint per page.
#[test]
fn unit_cost_per_recorded_save_is_one_blob_plus_one_index() {
    let dir = scratch("unit-cost");
    let ledger = files(&dir.join("ledger"));
    let page = |blocks: usize, edit: usize| {
        (0..blocks)
            .map(|i| {
                format!(
                    "- block {i} with some ordinary words in it {}\n",
                    if i == 0 { edit } else { 0 }
                )
            })
            .collect::<String>()
    };
    for blocks in [1usize, 60] {
        let rel = format!("pages/P{blocks}.md");
        for edit in 0..5 {
            ledger.record(&rel, page(blocks, edit).as_bytes()).unwrap();
        }
        let dir = ledger.page_dir(&rel);
        let before: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .sum();
        let text = page(blocks, 99);
        ledger.record(&rel, text.as_bytes()).unwrap();
        let index = std::fs::metadata(dir.join("index.json")).unwrap().len();
        let written = text.len() as u64 + index;
        let after: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .sum();
        println!(
            "UNIT-COST blocks={blocks} page_bytes={} written_bytes={written} files_written=2 \
             index_bytes={index} footprint_bytes={after} footprint_files={} (before {before})",
            text.len(),
            file_count(&dir)
        );
        assert_eq!(file_count(&dir), RETAINED + 1);
        assert!(index < 400, "index {index}");
        assert!(after <= RETAINED as u64 * text.len() as u64 + index + 64);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The exit drain is wired into the one place a run ends.
#[test]
fn quitting_drains_the_ledger_within_its_budget() {
    let lib = include_str!("lib.rs");
    let exit = lib
        .find("matches!(event, tauri::RunEvent::Exit)")
        .expect("RunEvent::Exit arm");
    let drain = lib
        .find("concord_ledger::drain_all_for_exit")
        .expect("exit drain call");
    assert!(
        drain > exit && drain - exit < 400,
        "drain must run inside the Exit arm"
    );
    // Draining an idle or never-started ledger returns at once.
    let dir = scratch("drain");
    let (slot, _sub) = open_slot(&dir, dir.join("appdata"));
    let started = Instant::now();
    assert!(slot
        .concord_ledger
        .get()
        .unwrap()
        .drain_for_exit(Instant::now() + EXIT_DRAIN_BUDGET));
    assert!(started.elapsed() <= EXIT_DRAIN_BUDGET);
    drop(slot);
    let _ = std::fs::remove_dir_all(dir);
}
