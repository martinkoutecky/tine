//! Device watch preference and the transport adapter for one graph window.

use crate::settings::{settings_path, update_settings};
use crate::state::{AppState, GraphSlot};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Instant, SystemTime};
use tauri::{Emitter, Manager, State};
use tine_core::model::PageKind;
use tine_store::{Change, ChangeKind, GraphRev, Origin, SubscriptionEnd, WatchBatch, WatchMode};

/// Above this many changed pages one publication is announced as ONE
/// `graph-changed-bulk` event (master 1229f32fb, GH #337): a checkout or big
/// sync otherwise costs the window one reload decision and one `dataRev` bump
/// per page. The store escalates its reconcile at the same boundary.
const BULK_CHANGE_THRESHOLD: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct GraphChange {
    path: String,
    name: String,
    kind: PageKind,
    created: bool,
    removed: bool,
}

pub(crate) fn watch_mode(app: &tauri::AppHandle) -> WatchMode {
    let selected = settings_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| serde_json::from_str::<serde_json::Value>(&source).ok())
        .and_then(|settings| settings.get("watch_mode")?.as_str().map(str::to_owned));
    selected_watch_mode(selected.as_deref())
}

/// The device's `watch_mode` setting → mode. Unset (or unknown) means native
/// events on every platform, Android included (master 7e1b6ec42, GH #337);
/// "poll" stays the opt-in for filesystems without reliable events, and the
/// store degrades to polling by itself when the OS refuses a watch.
fn selected_watch_mode(selected: Option<&str>) -> WatchMode {
    match selected {
        Some("poll") => WatchMode::Poll,
        _ => WatchMode::Notify,
    }
}

#[tauri::command]
pub(crate) fn get_watch_mode(app: tauri::AppHandle) -> String {
    match watch_mode(&app) {
        WatchMode::Notify => "inotify",
        WatchMode::Poll => "poll",
    }
    .into()
}

#[tauri::command]
pub(crate) fn set_watch_mode(
    mode: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mode = if mode == "poll" {
        WatchMode::Poll
    } else {
        WatchMode::Notify
    };
    update_settings(&app, |settings| {
        settings["watch_mode"] = serde_json::json!(match mode {
            WatchMode::Notify => "inotify",
            WatchMode::Poll => "poll",
        });
    })?;
    for (_, slot) in state.graphs.read().unwrap().entries() {
        slot.store.set_watch_mode(mode);
    }
    Ok(())
}

/// The v0.6.5 window events for one publication: `graph-changed` payloads in
/// file order, and whether `conflicts-changed` is due. Own changes emit none.
fn window_events(change: &Change) -> (Vec<GraphChange>, bool) {
    let mut events = Vec::new();
    let mut conflicts_dirty = false;
    if change.origin == Origin::Own {
        return (events, conflicts_dirty);
    }
    for (id, kind, _) in &change.files {
        if id.as_str().starts_with("assets/") {
            continue; // the asset lane (`asset_event_payload`), never a page
        }
        if tine_core::model::path_is_sync_conflict(Path::new(id.as_str())) {
            conflicts_dirty = true;
        } else if let Some((page_kind, name)) = change.page(id) {
            events.push(GraphChange {
                path: id.as_str().to_owned(),
                name: name.to_owned(),
                kind: page_kind,
                created: matches!(kind, ChangeKind::Created),
                removed: matches!(kind, ChangeKind::Removed),
            });
        }
    }
    (events, conflicts_dirty)
}

/// The `asset-changed` window event for one publication (master d017d1afc):
/// the assets-relative paths of files an outside actor created, replaced or
/// deleted, so the WebView drops its cached blobs. Own writes and page files
/// carry none, and no absolute path crosses the bridge.
fn asset_event_payload(change: &Change, binding_generation: u64) -> Option<serde_json::Value> {
    if change.origin != Origin::External {
        return None;
    }
    let mut paths: Vec<&str> = change
        .files
        .iter()
        .filter_map(|(id, _, _)| id.as_str().strip_prefix("assets/"))
        .collect();
    if paths.is_empty() {
        return None;
    }
    paths.sort_unstable();
    paths.dedup();
    Some(serde_json::json!({ "paths": paths, "binding_generation": binding_generation }))
}

/// Concord's share of one publication: the base ledger records it
/// off-thread (one channel send here) and the derived conflict queue
/// re-derives the entries it can affect. Returns whether the queue changed.
pub(crate) fn concord_observe(slot: &GraphSlot, change: &Change) -> bool {
    if let Some(ledger) = slot.concord_ledger.get() {
        ledger.observe(change);
    }
    match slot.conflict_queue.refresh_change(&slot.store, change) {
        Ok(changed) => changed,
        Err(error) => {
            crate::debug::diag_private("conflict-refresh-failed", error.to_string());
            true // notify clients to fetch the typed failure and keep their last inventory
        }
    }
}

// ---------------------------------------------------------------------------
// Watcher latency receipts (GH #337 diagnosis; master 388aede67)
// ---------------------------------------------------------------------------
// One receipt per external watcher publication: first notification ->
// reconcile start -> last window event emitted. A structured `--debug` line
// plus a 64-entry in-memory ring that `watcher_latency_recent` returns (a
// reporter runs it from the devtools console). No reads, no per-path data.

/// One external-change batch, as the watcher experienced it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct WatcherLatencyReceipt {
    /// Process-wide receipt number.
    seq: u64,
    /// Wall-clock Unix ms, to correlate with "I saved the file at ...".
    at_unix_ms: u64,
    /// Window page events emitted for this batch.
    pages: usize,
    #[serde(flatten)]
    timing: crate::flight::WatcherTiming,
}

const LATENCY_RECEIPT_CAP: usize = 64;
static LATENCY_RECEIPTS: Mutex<VecDeque<WatcherLatencyReceipt>> = Mutex::new(VecDeque::new());
static LATENCY_RECEIPT_SEQ: AtomicU64 = AtomicU64::new(0);

fn latency_receipt(batch: &WatchBatch, pages: usize, emitted_at: Instant) -> WatcherLatencyReceipt {
    let since = |earlier: Instant| emitted_at.saturating_duration_since(earlier).as_millis() as u64;
    WatcherLatencyReceipt {
        seq: 0,
        at_unix_ms: 0,
        pages,
        timing: crate::flight::WatcherTiming {
            mode: if batch.poll { "poll" } else { "inotify" },
            event_paths: batch.event_paths,
            full_diff: batch.full_diff,
            event_to_reconcile_ms: batch.first_event_at.map(|at| {
                batch
                    .reconcile_started
                    .saturating_duration_since(at)
                    .as_millis() as u64
            }),
            reconcile_ms: since(batch.reconcile_started),
            event_to_emit_ms: batch.first_event_at.map(since),
        },
    }
}

fn push_latency_receipt(
    ring: &mut VecDeque<WatcherLatencyReceipt>,
    receipt: WatcherLatencyReceipt,
) {
    while ring.len() >= LATENCY_RECEIPT_CAP {
        ring.pop_front();
    }
    ring.push_back(receipt);
}

fn record_latency_receipt(mut receipt: WatcherLatencyReceipt) {
    receipt.seq = LATENCY_RECEIPT_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    receipt.at_unix_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    let stage = |value: Option<u64>| value.map_or_else(|| "n/a".to_owned(), |ms| format!("{ms}ms"));
    crate::debug::diag_private(
        "watcher-latency",
        format!(
            "watcher-latency seq={} mode={} pages={} event_paths={} full_diff={} event->reconcile={} reconcile={}ms event->emit={}",
            receipt.seq,
            receipt.timing.mode,
            receipt.pages,
            receipt.timing.event_paths,
            receipt.timing.full_diff,
            stage(receipt.timing.event_to_reconcile_ms),
            receipt.timing.reconcile_ms,
            stage(receipt.timing.event_to_emit_ms),
        ),
    );
    if let Ok(mut ring) = LATENCY_RECEIPTS.lock() {
        push_latency_receipt(&mut ring, receipt);
    }
}

/// Debug command for bug reports: the last 64 external-change latency
/// receipts, oldest first.
#[tauri::command]
pub(crate) fn watcher_latency_recent() -> Vec<WatcherLatencyReceipt> {
    LATENCY_RECEIPTS
        .lock()
        .map(|ring| ring.iter().cloned().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Reload on focus (Concord L0; master d56219d73, b3d64addee39)
// ---------------------------------------------------------------------------
// Returning to the window asks for one full stat diff. It is not a second
// freshness path: whatever the diff finds is published by the store and
// emitted by the ordinary dispatch thread as `graph-changed` events. Once
// every event up to that publication has been emitted, `graph-rescan-complete`
// {sequence} follows on the same window-event channel, so the window can wait
// for its answer.

/// How far this binding's dispatch thread has emitted, and the rescans waiting
/// for it. One per `GraphSlot`.
#[derive(Default)]
pub(crate) struct RescanCursor {
    state: Mutex<(Option<GraphRev>, Vec<(GraphRev, u64)>)>,
}

static RESCAN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn emit_rescan_complete(app: &tauri::AppHandle, label: &str, sequence: u64) {
    let _ = app.emit_to(label, "graph-rescan-complete", sequence);
}

impl RescanCursor {
    /// The dispatch thread emitted every event up to publication `rev`;
    /// returns the rescans that are now complete.
    fn dispatched(&self, rev: GraphRev) -> Vec<u64> {
        let mut state = self.state.lock().unwrap();
        let done = state.0.map_or(rev, |seen| seen.max(rev));
        state.0 = Some(done);
        let mut complete = Vec::new();
        state.1.retain(|&(target, sequence)| {
            let finished = target <= done;
            if finished {
                complete.push(sequence);
            }
            !finished
        });
        complete
    }

    /// Whether `sequence` is already complete; otherwise it completes when
    /// publication `target` has been dispatched.
    fn wait(&self, target: GraphRev, sequence: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.0.is_some_and(|done| target <= done) {
            return true;
        }
        state.1.push((target, sequence));
        false
    }
}

/// Run one rescan for the calling window's graph and answer the sequence
/// number its `graph-rescan-complete` event will carry. By default it is the
/// full stat diff `Store::scan_refresh` documents (one stat per graph-text
/// file plus the bytes of changed files), which the focus return uses; with
/// `rebuild` (Settings "Rescan graph") it is `Store::rebuild_graph`, which
/// ignores every stamp and re-reads and re-parses every file. The scan runs
/// on the blocking pool; a failed scan still completes, and the watcher stays
/// primary.
#[tauri::command]
pub(crate) async fn rescan_graph_now(
    state: crate::state::GraphContext<'_>,
    rebuild: Option<bool>,
) -> Result<u64, String> {
    let slot = crate::state::slot_for_context(&state)?;
    let app = state.window.app_handle().clone();
    let label = state.window.label().to_owned();
    let sequence = RESCAN_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    tauri::async_runtime::spawn_blocking(move || {
        // A failed scan published nothing to wait for: it completes at once.
        let scanned = if rebuild.unwrap_or(false) {
            slot.store.rebuild_graph()
        } else {
            slot.store.scan_refresh()
        };
        let Ok(target) = scanned else {
            crate::debug::diag("watcher-focus-rescan-failed");
            return emit_rescan_complete(&app, &label, sequence);
        };
        if slot.rescan.wait(target, sequence) {
            emit_rescan_complete(&app, &label, sequence);
        }
    });
    Ok(sequence)
}

/// One publication's page events as window events: one `graph-changed` per
/// page, or a single `graph-changed-bulk` above `BULK_CHANGE_THRESHOLD`.
fn page_event_payloads(
    events: Vec<GraphChange>,
    binding_generation: u64,
    answers: Option<serde_json::Value>,
) -> Vec<(&'static str, serde_json::Value)> {
    let payload = |event: GraphChange| {
        serde_json::json!({
            "path": event.path,
            "name": event.name,
            "kind": event.kind,
            "created": event.created,
            "removed": event.removed,
            "binding_generation": binding_generation,
        })
    };
    if events.len() > BULK_CHANGE_THRESHOLD || events.is_empty() && answers.is_some() {
        let changes: Vec<_> = events.into_iter().map(payload).collect();
        let bulk = serde_json::json!({ "changes": changes, "binding_generation": binding_generation, "answers": answers });
        vec![("graph-changed-bulk", bulk)]
    } else {
        events
            .into_iter()
            .enumerate()
            .map(|(i, event)| {
                let mut value = payload(event);
                if i == 0 {
                    value["answers"] = serde_json::json!(answers);
                }
                ("graph-changed", value)
            })
            .collect()
    }
}

/// Emit one publication's window events; an external publication is also
/// recorded as a fixed-shape `watcher.batch` diagnostic event (counts only).
fn dispatch(app: &tauri::AppHandle, label: &str, slot: &GraphSlot, change: Change) {
    let binding_generation = slot.binding_generation;
    let config_changed = change.origin == Origin::External
        && change
            .files
            .iter()
            .any(|(id, _, _)| id.as_str() == "logseq/config.edn");
    let (events, copies_changed) = window_events(&change);
    let conflicts_dirty = concord_observe(slot, &change) || copies_changed;
    let pages = events.len();
    // Own publications also update derived answers for deletes/renames and
    // other windows. Empty text-only deltas emit no additional event.
    let answers = serde_json::to_value(&change).expect("Change answer serialization");
    let answers = (answers["inventoryChanged"] == true
        || answers["blockRefCounts"]
            .as_object()
            .is_some_and(|counts| !counts.is_empty()))
    .then_some(answers);
    for (name, payload) in page_event_payloads(events, binding_generation, answers) {
        let _ = app.emit_to(label, name, payload);
    }
    if let Some(payload) = asset_event_payload(&change, binding_generation) {
        let _ = app.emit_to(label, "asset-changed", payload);
    }
    // Measured once, after the last window event: the flight event and the
    // devtools ring report the same numbers.
    let receipt = change
        .watch
        .as_ref()
        .filter(|_| change.origin == Origin::External && (pages > 0 || !change.files.is_empty()))
        .map(|batch| latency_receipt(batch, pages, Instant::now()));
    if pages > 0 || conflicts_dirty {
        let timing = receipt.as_ref().map(|receipt| &receipt.timing);
        crate::flight::record_watcher_batch(pages, conflicts_dirty, timing);
    }
    if let Some(receipt) = receipt {
        record_latency_receipt(receipt);
    }
    if conflicts_dirty {
        let _ = app.emit_to(label, "conflicts-changed", ());
    }
    if config_changed {
        let _ = app.emit_to(
            label,
            "graph-config-changed",
            serde_json::json!({
                "binding_generation": binding_generation,
                "meta": crate::state::graph_meta(slot),
            }),
        );
    }
}

/// Tell the window when the OS refuses live notifications for its graph (the
/// store then polls every three seconds and retries), and when they return.
/// In-scope scenarios: inotify's per-user watch limit reached by a second
/// large graph, a network mount or filesystem without notifications.
fn report_watch_status(
    app: &tauri::AppHandle,
    label: &str,
    slot: &Weak<GraphSlot>,
    refusal: Option<String>,
) {
    let current = app.state::<AppState>().graphs.read().unwrap().slot(label);
    let Some(slot) = slot.upgrade() else {
        return;
    };
    if !current.is_some_and(|current| Arc::ptr_eq(&current, &slot)) {
        return;
    }
    let refused = refusal.is_some();
    crate::flight::record_watch_refused(refused);
    let (event, message) = match refusal {
        Some(message) => ("graph-watch-refused", message),
        None => ("graph-watch-restored", String::new()),
    };
    crate::debug::diag_private(event, format!("{event} {message}"));
    let _ = app.emit_to(
        label,
        event,
        serde_json::json!({ "message": message, "binding_generation": slot.binding_generation }),
    );
}

pub(crate) fn start_slot_events(app: tauri::AppHandle, label: String, slot: &Arc<GraphSlot>) {
    let subscription = slot.store.subscribe();
    let weak: Weak<GraphSlot> = Arc::downgrade(slot);
    slot.rescan.dispatched(subscription.start_rev());
    {
        let (app, label, weak) = (app.clone(), label.clone(), weak.clone());
        subscription
            .observe_watch_status(move |status| report_watch_status(&app, &label, &weak, status));
    }
    std::thread::spawn(move || loop {
        let change = match subscription.recv() {
            Ok(change) => change,
            Err(SubscriptionEnd::StoreClosed) => break,
            Err(SubscriptionEnd::Displaced) => {
                crate::debug::diag("watch-subscription-displaced");
                break;
            }
        };
        let Some(slot) = weak.upgrade() else {
            break;
        };
        let current = app.state::<AppState>().graphs.read().unwrap().slot(&label);
        if current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &slot))
        {
            let rev = change.graph_rev;
            dispatch(&app, &label, &slot, change);
            for sequence in slot.rescan.dispatched(rev) {
                emit_rescan_complete(&app, &label, sequence);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(value: u64) -> GraphRev {
        GraphRev::try_from(value.to_string()).unwrap()
    }

    #[test]
    fn derived_answers_travel_once_per_single_or_bulk_publication() {
        let answers =
            serde_json::json!({"rev":"9", "inventoryChanged":true, "blockRefCounts":{"target":3}});
        let single = page_event_payloads(vec![page(0), page(1)], 7, Some(answers.clone()));
        assert_eq!(single[0].1["answers"], answers);
        assert!(single[1].1.get("answers").is_none());
        let bulk = page_event_payloads((0..40).map(page).collect(), 7, Some(answers.clone()));
        assert_eq!(bulk[0].1["answers"], answers);
        assert!(bulk[0].1["changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c.get("answers").is_none()));
        let own = page_event_payloads(vec![], 7, Some(answers.clone()));
        assert_eq!(own[0].1["answers"], answers);
        assert_eq!(own[0].1["changes"], serde_json::json!([]));
        assert!(page_event_payloads(vec![], 7, None).is_empty());
    }

    fn page(index: usize) -> GraphChange {
        GraphChange {
            path: format!("pages/p{index}.md"),
            name: format!("p{index}"),
            kind: PageKind::Page,
            created: false,
            removed: false,
        }
    }

    /// og-T3 (master 7e1b6ec42): the default is native events on every
    /// platform. The source check is the platform observer: a per-target
    /// default compiles away on the Linux test host.
    #[test]
    fn unset_watch_mode_prefers_native_events_on_every_platform() {
        assert_eq!(selected_watch_mode(None), WatchMode::Notify);
        assert_eq!(selected_watch_mode(Some("inotify")), WatchMode::Notify);
        assert_eq!(selected_watch_mode(Some("poll")), WatchMode::Poll);
        let source = include_str!("watcher.rs");
        let selection = &source[source.find("fn watch_mode(").unwrap()..];
        let selection = &selection[..selection.find("#[tauri::command]").unwrap()];
        assert!(
            !selection.contains("target_os"),
            "the watch-mode default must not vary by platform (master 7e1b6ec42)"
        );
    }

    #[test]
    fn a_checkout_sized_publication_is_one_bulk_window_event() {
        let few = page_event_payloads((0..BULK_CHANGE_THRESHOLD).map(page).collect(), 7, None);
        assert_eq!(few.len(), BULK_CHANGE_THRESHOLD);
        assert!(few.iter().all(|(name, _)| *name == "graph-changed"));
        let many = page_event_payloads((0..=BULK_CHANGE_THRESHOLD).map(page).collect(), 7, None);
        assert_eq!(many.len(), 1, "one event for a checkout-sized batch");
        let (name, payload) = &many[0];
        assert_eq!(*name, "graph-changed-bulk");
        assert_eq!(
            payload["changes"].as_array().unwrap().len(),
            BULK_CHANGE_THRESHOLD + 1
        );
        assert_eq!(payload["binding_generation"], 7);
        assert_eq!(payload["changes"][3]["name"], "p3");
        assert_eq!(payload["changes"][3]["binding_generation"], 7);
    }

    #[test]
    fn a_focus_rescan_completes_only_after_its_publication_is_dispatched() {
        let cursor = RescanCursor::default();
        assert!(cursor.dispatched(rev(4)).is_empty());
        assert!(
            cursor.wait(rev(4), 1),
            "nothing new published: complete at once"
        );
        assert!(
            !cursor.wait(rev(6), 2),
            "the rescan's publication is not emitted yet"
        );
        assert!(cursor.dispatched(rev(5)).is_empty());
        assert_eq!(cursor.dispatched(rev(6)), vec![2]);
        assert!(cursor.dispatched(rev(7)).is_empty(), "completes once");
    }

    #[test]
    fn latency_receipts_keep_the_last_sixty_four() {
        let started = Instant::now();
        let batch = WatchBatch {
            first_event_at: Some(started),
            reconcile_started: started + std::time::Duration::from_millis(200),
            poll: false,
            full_diff: false,
            event_paths: 2,
        };
        let receipt = latency_receipt(&batch, 1, started + std::time::Duration::from_millis(250));
        assert_eq!(receipt.timing.mode, "inotify");
        assert_eq!(receipt.timing.event_to_reconcile_ms, Some(200));
        assert_eq!(receipt.timing.reconcile_ms, 50);
        assert_eq!(receipt.timing.event_to_emit_ms, Some(250));
        let poll = WatchBatch {
            first_event_at: None,
            poll: true,
            full_diff: true,
            ..batch
        };
        assert_eq!(
            latency_receipt(&poll, 0, started).timing.event_to_emit_ms,
            None
        );
        let mut ring = VecDeque::new();
        for seq in 0..70 {
            push_latency_receipt(
                &mut ring,
                WatcherLatencyReceipt {
                    seq,
                    ..receipt.clone()
                },
            );
        }
        assert_eq!(ring.len(), LATENCY_RECEIPT_CAP);
        assert_eq!(ring.front().unwrap().seq, 6);
        assert_eq!(ring.back().unwrap().seq, 69);
    }

    /// Scan-refresh an external edit and return the window events it yields.
    fn events_after(slot: &GraphSlot, edit: impl FnOnce()) -> Vec<GraphChange> {
        slot.store.whole_graph().unwrap();
        let subscription = slot.store.subscribe();
        edit();
        slot.store.scan_refresh().unwrap();
        let mut events = Vec::new();
        while let Some(change) = subscription.try_recv().unwrap() {
            events.extend(window_events(&change).0);
        }
        events
    }

    fn atomic_write(root: &Path, rel: &str, text: &str) {
        let temp = root.join(".adapter-write");
        std::fs::write(&temp, text).unwrap();
        std::fs::rename(temp, root.join(rel)).unwrap();
    }

    #[test]
    fn graph_changed_uses_the_store_page_name() {
        let root = std::env::temp_dir().join(format!(
            "tine-watch-names-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::write(root.join("pages/foo.md"), "title:: Bar\n\n- one\n").unwrap();
        std::fs::write(root.join("journals/2026_07_10.md"), "- day\n").unwrap();
        std::fs::write(root.join("journals/Jul 10th, 2026.md"), "- shadow\n").unwrap();
        let store = tine_store::Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Poll,
                launch_checkpoint: None,
            },
        )
        .unwrap()
        .0;
        let slot = GraphSlot::new(store, root.clone());
        let modified = |name: &str, path: &str, kind| GraphChange {
            path: path.into(),
            name: name.into(),
            kind,
            created: false,
            removed: false,
        };

        let events = events_after(&slot, || {
            atomic_write(&root, "pages/foo.md", "title:: Bar\n\n- two, longer\n")
        });
        assert_eq!(
            events,
            vec![modified("Bar", "pages/foo.md", PageKind::Page)]
        );

        let events = events_after(&slot, || {
            atomic_write(&root, "journals/Jul 10th, 2026.md", "- shadow edited\n")
        });
        assert_eq!(events, vec![], "a shadow journal is not a graph page");

        let events = events_after(&slot, || {
            atomic_write(&root, "journals/2026_07_10.md", "- day two\n")
        });
        assert_eq!(
            events,
            vec![modified(
                "Jul 10th, 2026",
                "journals/2026_07_10.md",
                PageKind::Journal
            )]
        );

        let events = events_after(&slot, || {
            std::fs::remove_file(root.join("pages/foo.md")).unwrap()
        });
        assert_eq!(
            events,
            vec![GraphChange {
                path: "pages/foo.md".into(),
                name: "Bar".into(),
                kind: PageKind::Page,
                created: false,
                removed: true,
            }]
        );
        drop(slot);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_page_changes_keep_the_v065_payload_shape() {
        let root = std::env::temp_dir().join(format!(
            "tine-watch-adapter-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        let store = tine_store::Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Poll,
                launch_checkpoint: None,
            },
        )
        .unwrap()
        .0;
        let slot = GraphSlot::new(store, root.clone());
        let event = |created, removed| GraphChange {
            path: "pages/New.md".into(),
            name: "New".into(),
            kind: PageKind::Page,
            created,
            removed,
        };
        assert_eq!(
            events_after(&slot, || atomic_write(&root, "pages/New.md", "- new\n")),
            vec![event(true, false)]
        );
        let file = std::fs::File::options()
            .write(true)
            .open(root.join("pages/New.md"))
            .unwrap();
        assert_eq!(
            events_after(&slot, || file
                .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(5))
                .unwrap()),
            vec![],
            "a touch publishes a generation but no window event"
        );
        assert_eq!(
            events_after(&slot, || std::fs::remove_file(root.join("pages/New.md"))
                .unwrap()),
            vec![event(false, true)]
        );
        atomic_write(&root, "pages/New.md", "title:: New\n\n- winner\n");
        slot.store.scan_refresh().unwrap();
        slot.store.whole_graph().unwrap();
        let subscription = slot.store.subscribe();
        atomic_write(
            &root,
            "pages/New.sync-conflict-20260705-120000-ABCDEFG.md",
            "title:: New\n\n- theirs\n",
        );
        slot.store.scan_refresh().unwrap();
        let change = subscription
            .try_recv()
            .unwrap()
            .expect("conflict copy publishes");
        assert_eq!(window_events(&change), (vec![], true));
        assert!(matches!(
            slot.store.whole_graph().unwrap().resolve("New", false),
            tine_store::Resolved::Existing { id, others }
                if id.as_str() == "pages/New.md" && others.is_empty()
        ));
        assert!(
            tine_graph_features::conflicts::list_sync_conflicts(&slot.store)
                .unwrap()
                .iter()
                .any(|copy| copy.path == "pages/New.sync-conflict-20260705-120000-ABCDEFG.md")
        );
        drop(slot);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_external_bytes_dispatch_graph_changed() {
        use tine_store::{FaultPoint, PageId, SaveBase, TxOutcome};

        let root = std::env::temp_dir().join(format!(
            "tine-watch-rollback-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/A.md"), "- old A\n").unwrap();
        std::fs::write(root.join("pages/B.md"), "- old B\n").unwrap();
        let store = tine_store::Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Poll,
                launch_checkpoint: None,
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        let subscription = store.subscribe();
        let a = PageId::from("pages/A.md");
        let b = PageId::from("pages/B.md");
        let read_a = store.page(&a).unwrap();
        let read_b = store.page(&b).unwrap();
        let mut doc_a = read_a.doc;
        let mut doc_b = read_b.doc;
        doc_a.blocks[0].raw = "new A".into();
        doc_b.blocks[0].raw = "new B".into();
        let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
        tx.save_page(
            &[tine_store::EditKind::ReplacePage],
            &a,
            SaveBase::Existing(read_a.rev),
            &doc_a,
        );
        tx.save_page(
            &[tine_store::EditKind::ReplacePage],
            &b,
            SaveBase::Existing(read_b.rev),
            &doc_b,
        );
        store.inject_fault(FaultPoint::MidStepIoAt(1));
        store.inject_fault(FaultPoint::UndoLiveWrite);
        assert!(matches!(tx.commit(), TxOutcome::NotCommitted { .. }));
        let events: Vec<_> = std::iter::from_fn(|| subscription.try_recv().unwrap())
            .flat_map(|change| window_events(&change).0)
            .collect();
        assert_eq!(
            events,
            vec![GraphChange {
                path: "pages/B.md".into(),
                name: "B".into(),
                kind: PageKind::Page,
                created: false,
                removed: false,
            }]
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// og-J2 (master d017d1afc): an outside asset replace/create/delete is the
    /// window's `asset-changed` event with assets-relative paths and the
    /// binding generation; own writes, page files, and asset names that look
    /// like sync-conflict copies produce no page event and no conflicts flag.
    #[test]
    fn an_external_asset_change_is_one_asset_changed_event() {
        let root = std::env::temp_dir().join(format!(
            "tine-watch-assets-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("assets/sub")).unwrap();
        std::fs::write(root.join("assets/pic.png"), b"one").unwrap();
        let store = tine_store::Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Poll,
                launch_checkpoint: None,
            },
        )
        .unwrap()
        .0;
        let slot = GraphSlot::new(store, root.clone());
        slot.store.whole_graph().unwrap();
        let subscription = slot.store.subscribe();
        atomic_write(&root, "assets/pic.png", "two, longer");
        atomic_write(
            &root,
            "assets/sub/pic.sync-conflict-20260705-120000-ABCDEFG.png",
            "x",
        );
        atomic_write(&root, "pages/P.md", "- page\n");
        tine_graph_features::assets::save_asset(&slot.store, "own.png", b"mine").unwrap();
        slot.store.scan_refresh().unwrap();
        let (mut payloads, mut page_events, mut conflicts) = (Vec::new(), 0, false);
        while let Some(change) = subscription.try_recv().unwrap() {
            let (events, dirty) = window_events(&change);
            page_events += events.len();
            conflicts |= dirty;
            payloads.extend(asset_event_payload(&change, 7));
        }
        // The background poll cycle may publish part of the batch first; the
        // contract is one event per publication, each asset reported once, the
        // own write never, and the graph binding on every payload.
        assert!(payloads
            .iter()
            .all(|payload| payload["binding_generation"] == 7));
        let mut paths: Vec<_> = payloads
            .iter()
            .flat_map(|payload| payload["paths"].as_array().unwrap().iter())
            .map(|path| path.as_str().unwrap().to_owned())
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            [
                "pic.png",
                "sub/pic.sync-conflict-20260705-120000-ABCDEFG.png"
            ]
        );
        assert_eq!(page_events, 1, "only pages/P.md is a page event");
        assert!(!conflicts, "an asset is never a conflict copy");
        drop(slot);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Concord incremental marker check (Martin-approved, og 20a): a marker
    /// page written while the graph is open reaches the derived queue through
    /// the real device watcher's next external event, re-deriving only the
    /// changed file (the one full walk happened at open), and leaves it when
    /// an outside tool resolves the markers. The answer equals a full walk.
    #[test]
    fn a_marker_written_while_open_enters_the_queue_on_its_next_external_event() {
        let root = std::env::temp_dir().join(format!(
            "tine-watch-markers-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::write(root.join("pages/Plain.md"), "- plain\n").unwrap();
        let store = tine_store::Store::open(
            &root,
            tine_store::OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Notify,
                launch_checkpoint: None,
            },
        )
        .unwrap()
        .0;
        let slot = GraphSlot::new(store, root.clone());
        slot.store.whole_graph().unwrap();
        assert!(slot
            .conflict_queue
            .inventory(&slot.store)
            .unwrap()
            .queue
            .is_empty());
        let subscription = slot.store.subscribe();
        // Deliver external changes as the dispatch thread does until `rel`
        // was among them; answers whether the queue changed.
        let deliver = |rel: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let mut dirty = false;
            loop {
                match subscription.try_recv().unwrap() {
                    Some(change) => {
                        assert_eq!(change.origin, tine_store::Origin::External);
                        dirty |= concord_observe(&slot, &change);
                        if change.files.iter().any(|(id, _, _)| id.as_str() == rel) {
                            return dirty;
                        }
                    }
                    None if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(20))
                    }
                    None => panic!("the watcher never published {rel}"),
                }
            }
        };
        let marked = "- before\n<<<<<<< HEAD\n- mine\n=======\n- theirs\n>>>>>>> other\n";
        atomic_write(&root, "pages/Marked.md", marked);
        assert!(deliver("pages/Marked.md"), "the queue changed");
        let queued = slot.conflict_queue.inventory(&slot.store).unwrap();
        assert_eq!(
            queued
                .vcs_markers
                .iter()
                .map(|m| m.path.as_str())
                .collect::<Vec<_>>(),
            vec!["pages/Marked.md"]
        );
        assert_eq!(queued.queue.len(), 1);
        assert_eq!(
            serde_json::to_string(&queued).unwrap(),
            serde_json::to_string(
                &tine_graph_features::conflicts::conflict_inventory(&slot.store).unwrap()
            )
            .unwrap(),
            "the incremental answer equals a full walk"
        );
        // An unrelated external edit re-derives only itself: no change.
        atomic_write(&root, "pages/Plain.md", "- plain edited\n");
        assert!(!deliver("pages/Plain.md"));
        // git resolved the markers outside Tine: the entry leaves.
        atomic_write(&root, "pages/Marked.md", "- before\n- merged\n");
        assert!(deliver("pages/Marked.md"));
        assert!(slot
            .conflict_queue
            .inventory(&slot.store)
            .unwrap()
            .queue
            .is_empty());
        drop(subscription);
        drop(slot);
        std::fs::remove_dir_all(root).unwrap();
    }
}
