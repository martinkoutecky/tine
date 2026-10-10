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
            launch_checkpoint: None,
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
    slot.store.refresh(tine_store::Depth::Stamps).unwrap();
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
    slot.conflict_queue.inventory(&slot.store).unwrap();
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
            .unwrap()
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
        None,
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
    assert!(slot
        .conflict_queue
        .inventory(&slot.store)
        .unwrap()
        .queue
        .is_empty());
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

/// The copy-equal artifact (why og's `pick_base` departs from master's
/// winner-only rule): the page was saved once by Tine on this device
/// ("Desktop"); the other device's edit ("Desktop 5 kk") wins the winner name
/// and Syncthing renames this device's bytes to the copy, in one scan. No pin
/// exists (the only retained text equals the copy), and after the admission the
/// winner's retained texts are [delivered, this device's save]. Master's rule
/// would take this device's save — the copy's own bytes — as the ancestor and
/// pre-select "mine" (the delivered text) on the row both devices edited,
/// discarding this device's edit. og keeps that review 2-way instead.
#[test]
fn a_base_equal_to_the_copys_own_bytes_is_not_used_as_the_ancestor() {
    let dir = scratch("copy-artifact");
    std::fs::write(dir.join("graph/pages/Desk.md"), body("seed")).unwrap();
    let (slot, sub) = open_slot(&dir, dir.join("appdata"));
    save(&slot, "pages/Desk.md", "Desktop");
    pump(&slot, &sub);
    std::fs::write(dir.join("graph").join(COPY), body("Desktop")).unwrap();
    external(&slot, "pages/Desk.md", &body("Desktop 5 kk"));
    pump(&slot, &sub);
    let ledger = slot.concord_ledger.get().unwrap();
    assert_eq!(ledger.files().pinned(COPY), None);
    let bases = ledger.conflict_bases(COPY, "pages/Desk.md");
    assert_eq!(bases, vec![body("Desktop 5 kk"), body("Desktop")]);
    let diff = conflicts::sync_conflict_diff(&slot.store, "pages/Desk.md", COPY, &bases)
        .unwrap()
        .unwrap();
    assert!(
        diff.rows
            .iter()
            .all(|row| row.suggestion.as_deref() != Some("mine")),
        "no row may pre-select discarding this device's edit: {:?}",
        diff.rows
    );
    assert!(!diff.three_way && diff.merge_base_rev.is_none());
    drop(slot);
    let _ = std::fs::remove_dir_all(dir);
}

/// A genuine ancestor equal to the copy is indistinguishable from the
/// artifact above: the review stays 2-way rather than suggesting "mine"
/// everywhere, and it never falls back to an older base, which could turn a
/// winner-side revert into a "theirs" suggestion.
#[test]
fn a_copy_equal_to_the_newest_base_stays_two_way_without_reaching_older_bases() {
    let dir = scratch("copy-equals-base");
    std::fs::write(dir.join("graph/pages/Desk.md"), body("seed")).unwrap();
    let (slot, _sub) = open_slot(&dir, dir.join("appdata"));
    std::fs::write(dir.join("graph/pages/Desk.md"), body("reverted")).unwrap();
    std::fs::write(dir.join("graph").join(COPY), body("ancestor")).unwrap();
    slot.store.refresh(tine_store::Depth::Stamps).unwrap();
    let bases = vec![body("ancestor"), body("reverted-from")];
    let diff = conflicts::sync_conflict_diff(&slot.store, "pages/Desk.md", COPY, &bases)
        .unwrap()
        .unwrap();
    assert!(!diff.three_way && diff.merge_base_rev.is_none(), "{diff:?}");
    assert!(diff.rows.iter().all(|row| row.suggestion.is_none()));
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
        None,
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
        None,
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
        None,
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
        None,
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
        let listing = |dir: &std::path::Path| -> std::collections::BTreeSet<String> {
            std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        };
        // The index's file identity: an atomic rewrite renames a new file in.
        // Unix only; elsewhere the listing assertions still run.
        let index_inode = || -> u64 {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                std::fs::metadata(dir.join("index.json")).unwrap().ino()
            }
            #[cfg(not(unix))]
            {
                0
            }
        };
        let (names_before, inode_before) = (listing(&dir), index_inode());
        let text = page(blocks, 99);
        ledger.record(&rel, text.as_bytes()).unwrap();
        // Work count (og C, I-25 wording): exactly one new blob, one evicted
        // blob removed, and one index rewrite (atomic rename = new inode).
        let names_after = listing(&dir);
        assert_eq!(
            names_after
                .difference(&names_before)
                .cloned()
                .collect::<Vec<_>>(),
            vec![sha(text.as_bytes())]
        );
        assert_eq!(names_before.difference(&names_after).count(), 1);
        if cfg!(unix) {
            assert_ne!(index_inode(), inode_before, "the index is rewritten once");
        }
        // Re-recording the newest text writes nothing.
        let inode_recorded = index_inode();
        ledger.record(&rel, text.as_bytes()).unwrap();
        assert_eq!(index_inode(), inode_recorded);
        assert_eq!(listing(&dir), names_after);
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
        // The measured row the module doc and ADR 0056 quote.
        assert_eq!(written, if blocks == 1 { 220 } else { 2_808 });
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

/// Every file under `dir` with its bytes, sorted: a byte-exact tree snapshot.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(tree(&path));
        } else {
            out.push((path.clone(), std::fs::read(&path).unwrap()));
        }
    }
    out.sort();
    out
}

/// og and master share one app-data directory at the identity flip (and on a
/// rollback). Master's ledger lives in `concord-ledger/` with a layout og does
/// not read; og must neither prune nor write it, and its own entries live in
/// [`LEDGER_DIR`] under the same root id.
#[test]
fn a_master_layout_ledger_tree_is_byte_identical_after_og_opens_saves_and_prunes() {
    let dir = scratch("master-tree");
    std::fs::write(dir.join("graph/pages/Desk.md"), body("base")).unwrap();
    let app_data = dir.join("appdata");
    let root_id = crate::backup::root_backup_id(&dir.join("graph"));
    let master = app_data.join("concord-ledger").join(&root_id);
    let master_page = master.join("pages").join(sha(b"pages/Desk.md"));
    std::fs::create_dir_all(&master_page).unwrap();
    std::fs::write(master_page.join("index.json"), br#"{"v":7,"heads":["x"]}"#).unwrap();
    std::fs::write(master_page.join("x"), b"- master text\n").unwrap();
    std::fs::create_dir_all(master.join("pins")).unwrap();
    std::fs::write(master.join("pins/orphan.json"), b"{}").unwrap();
    std::fs::write(master.join("stray-temp.tmp"), b"torn").unwrap();
    let before = tree(&app_data.join("concord-ledger"));

    let (slot, subscription) = open_slot(&dir, app_data.clone());
    save(&slot, "pages/Desk.md", "mine");
    pump(&slot, &subscription);
    let ledger = slot.concord_ledger.get().unwrap();
    assert!(ledger.files().prune(&slot.store).is_ok());
    assert_eq!(ledger.files().retained("pages/Desk.md")[0], body("mine"));
    assert!(ledger
        .dir
        .starts_with(app_data.join(LEDGER_DIR).join(&root_id)));

    assert_eq!(tree(&app_data.join("concord-ledger")), before);
    drop(slot);
    let _ = std::fs::remove_dir_all(dir);
}

/// og 8e through the ledger the commands read: the editor loaded "Desktop 5"
/// (a Tine save the ledger recorded) and deleted " 5"; another editor appended
/// " kk" on disk. The live review finds the editor's base by revision, is
/// 3-way with a `merged` proposal, and the resolve writes the composed body at
/// the reviewed disk revision. With an unusable ledger the same review is
/// 2-way and still resolves (the ledger never blocks a resolve).
#[test]
fn a_live_draft_conflict_reviews_three_way_against_the_editors_ledger_base() {
    use tine_graph_features::live_conflict::{live_conflict_diff, resolve_live_conflict};
    for (label, usable) in [("live-ledger", true), ("live-no-ledger", false)] {
        let dir = scratch(label);
        std::fs::write(dir.join("graph/pages/Desk.md"), body("seed")).unwrap();
        let app_data = if usable {
            dir.join("appdata")
        } else {
            std::fs::write(dir.join("appdata-file"), "not a directory").unwrap();
            dir.join("appdata-file")
        };
        let (slot, sub) = open_slot(&dir, app_data);
        save(&slot, "pages/Desk.md", "Desktop 5");
        pump(&slot, &sub);
        let read = slot.store.page(&PageId::from("pages/Desk.md")).unwrap();
        let base_rev: String = read.rev.into();
        let mut draft = read.doc;
        draft.blocks[1].raw = format!("Desktop\nid:: {ID}");
        external(&slot, "pages/Desk.md", &body("Desktop 5 kk"));
        pump(&slot, &sub);
        let bases = crate::concord::page_bases(&slot, "pages/Desk.md");
        let diff = live_conflict_diff(
            &slot.store,
            "pages/Desk.md",
            &draft,
            Some(&base_rev),
            &bases,
        )
        .unwrap();
        assert_eq!(diff.three_way, usable, "{label}");
        let decisions = preselected(&diff);
        assert_eq!(decisions.values().any(|d| d == "merged"), usable, "{label}");
        let resolved = resolve_live_conflict(
            &slot.store,
            None,
            "pages/Desk.md",
            &draft,
            Some(&base_rev),
            &diff.conflict_rev,
            diff.merge_base_rev.as_deref(),
            &bases,
            &decisions,
            "union",
        )
        .unwrap();
        let written = std::fs::read_to_string(dir.join("graph/pages/Desk.md")).unwrap();
        if usable {
            assert_eq!(written, body("Desktop kk"));
        } else {
            for text in ["- Desktop\n", "Desktop 5 kk"] {
                assert!(written.contains(text), "{text:?} lost: {written}");
            }
        }
        assert!(resolved.rev.is_some());
        drop(slot);
        let _ = std::fs::remove_dir_all(dir);
    }
}
