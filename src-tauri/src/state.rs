use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::ipc::{CommandArg, CommandItem, InvokeBody, InvokeError};
use tauri::{Runtime, State, WebviewWindow};
use tine_store::Store;

pub(crate) type WindowKey = String;

/// Read-only graph lease used by the auxiliary Quick Capture WebView. Capture
/// deliberately does not own a graph slot: the registry permits one writable
/// window per graph root, while this surface only needs the selected graph's
/// query/read commands before it hands writes back to the owning window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureGraphBinding {
    pub(crate) target: WindowKey,
    pub(crate) binding_generation: u64,
}

pub(crate) struct GraphSlot {
    /// Shared with the binding's page host. An adopting binding takes it
    /// over (`HostRetirement::adopt`), and this slot then leaves it open.
    pub(crate) store: Arc<Store>,
    /// Latest `((` request per transport lane for this window binding.
    pub(crate) block_search_lanes: tine_graph_features::search::SearchLanes,
    pub(crate) root_key: PathBuf,
    /// Unique lease for this exact window→graph binding. Frontend mutations carry
    /// it so an IPC queued before an in-place graph switch cannot execute against
    /// the replacement graph after the window label is rebound.
    pub(crate) binding_generation: u64,
    startup_idle: Mutex<Option<Instant>>,
    startup_changed: Condvar,
    pub(crate) warm_done: AtomicBool,
    pub(crate) warm_generation: AtomicU64,
    /// Revoked as soon as this exact window→graph binding is replaced/removed.
    /// Detached warm/backup workers check it before and during graph-sized work.
    pub(crate) background_cancelled: AtomicBool,
    /// Concord's derived conflict queue (memory only; see `ConflictQueue`).
    pub(crate) conflict_queue: tine_graph_features::conflicts::ConflictQueue,
    /// Concord base ledger, attached at graph open (`concord_ledger::attach`);
    /// empty when app data is unavailable, which only means 2-way reviews.
    pub(crate) concord_ledger: std::sync::OnceLock<crate::concord_ledger::ConcordLedger>,
    /// Focus rescans waiting for this binding's dispatch thread (family 10).
    pub(crate) rescan: crate::watcher::RescanCursor,
    /// The binding's page host (STEP3 §6–7). Census writers run under its
    /// reservations while one runs (`running`). Its write lock is the
    /// binding's host transition gate (plan v3 S8): a backup restore holds
    /// it across stop, file restore and relaunch, and retirement and
    /// adoption take it, so each waits for the others' outcome.
    ///
    /// Lock order (REVIEW-3b-P1 B1): registry, then this gate, then the
    /// retirement map, then the host's own state. Nothing holding this gate
    /// takes the registry; a writer checks revocation from the gate's own
    /// state (`host_slot`).
    pub(crate) host: RwLock<PageHostSlot>,
}

/// Where the binding's page host stands. `Off` until lane 3b's switch
/// starts one.
#[derive(Default)]
pub(crate) enum PageHostSlot {
    #[default]
    Off,
    Running(tine_store::PageHost),
    /// An open adopted this binding's Store and host
    /// (`HostRetirement::adopt`): a worker that captured the slot before
    /// gets the stale-binding outcome, never the no-host arm on the adopted
    /// Store (REVIEW-3b-P1 B1), and the slot no longer closes the Store.
    Revoked,
}

impl PageHostSlot {
    /// The running host, which census writers reserve their pages from.
    /// Reached only through `GraphSlot::host_slot`, which refuses a revoked
    /// slot.
    pub(crate) fn running(&self) -> Option<&tine_store::PageHost> {
        match self {
            Self::Running(host) => Some(host),
            Self::Off | Self::Revoked => None,
        }
    }
}

/// A revoked slot's outcome: the existing stale-binding refusal (scenario:
/// a command queued before its window closed and the root was reopened).
pub(crate) const STALE_BINDING: &str = "stale-graph-binding";

impl GraphSlot {
    pub(crate) fn new(store: impl Into<Arc<Store>>, root_key: PathBuf) -> Self {
        static NEXT_BINDING: AtomicU64 = AtomicU64::new(1);
        Self {
            store: store.into(),
            block_search_lanes: tine_graph_features::search::SearchLanes::default(),
            root_key,
            binding_generation: NEXT_BINDING.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            startup_idle: Mutex::new(None),
            startup_changed: Condvar::new(),
            warm_done: AtomicBool::new(false),
            warm_generation: AtomicU64::new(0),
            background_cancelled: AtomicBool::new(false),
            conflict_queue: Default::default(),
            concord_ledger: Default::default(),
            rescan: Default::default(),
            host: Default::default(),
        }
    }

    /// The host slot, read for a census writer's whole call: a restore
    /// cannot take the host out from under the writer's reservation, and a
    /// writer waits for a restore's outcome (S8). A revoked slot refuses
    /// here, under the gate, before the writer starts (B1).
    pub(crate) fn host_slot(&self) -> Result<std::sync::RwLockReadGuard<'_, PageHostSlot>, String> {
        let slot = self.host.read().unwrap();
        if matches!(*slot, PageHostSlot::Revoked) {
            return Err(STALE_BINDING.into());
        }
        Ok(slot)
    }

    pub(crate) fn begin_startup_warm(&self) -> u64 {
        let mut idle = self.startup_idle.lock().unwrap();
        *idle = None;
        self.warm_done
            .store(false, std::sync::atomic::Ordering::Release);
        let generation = self
            .warm_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        self.startup_changed.notify_all();
        generation
    }

    pub(crate) fn finish_startup_warm(&self, generation: u64) -> bool {
        let mut idle = self.startup_idle.lock().unwrap();
        if self
            .warm_generation
            .load(std::sync::atomic::Ordering::Acquire)
            != generation
        {
            return false;
        }
        self.warm_done
            .store(true, std::sync::atomic::Ordering::Release);
        *idle = Some(Instant::now());
        self.startup_changed.notify_all();
        true
    }

    pub(crate) fn cancel_background(&self) {
        let _idle = self.startup_idle.lock().unwrap();
        self.background_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.startup_changed.notify_all();
    }

    /// Wait off the UI thread for this binding's warm completion plus quiet,
    /// or the safety-net deadline. Cancellation wakes immediately and returns
    /// false. O(1), no graph reads; completion/reset and revocation signal the
    /// same condition, so neither an early signal nor a graph switch is lost.
    pub(crate) fn wait_startup_idle(&self, quiet: Duration, deadline: Duration) -> bool {
        let deadline = Instant::now() + deadline;
        let mut idle = self.startup_idle.lock().unwrap();
        loop {
            if self
                .background_cancelled
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return false;
            }
            let due = idle.map_or(deadline, |done| (done + quiet).min(deadline));
            let now = Instant::now();
            if now >= due {
                return true;
            }
            idle = self
                .startup_changed
                .wait_timeout(idle, due - now)
                .unwrap()
                .0;
        }
    }
}

pub(crate) fn graph_meta(slot: &GraphSlot) -> tine_core::model::GraphMeta {
    let config = slot.store.config();
    let format = tine_core::date::JournalFormat::new(
        config.journal_file_name_format.as_deref(),
        config.journal_page_title_format.as_deref(),
    );
    tine_core::model::GraphMeta::from_config(
        slot.root_key.display().to_string(),
        &config.config,
        &format,
    )
}

/// Test probe run as a slot starts closing, with that slot's root, so a test
/// can observe which locks are held while the Store closes.
#[cfg(test)]
#[allow(clippy::type_complexity)]
pub(crate) static SLOT_CLOSE_PROBE: Mutex<Option<Box<dyn Fn(&Path) + Send>>> = Mutex::new(None);

impl Drop for GraphSlot {
    /// The host stops before its Store closes (plan v3 §3). A slot that
    /// held a running host was retired first (`HostRetirement`), so a host
    /// still here is a test fixture's or an unbound open's: dropping it is a
    /// crash stop, which keeps every written draft.
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(probe) = SLOT_CLOSE_PROBE.lock().unwrap().as_ref() {
            probe(&self.root_key);
        }
        let host = self.host.get_mut().unwrap_or_else(|e| e.into_inner());
        if !matches!(std::mem::take(host), PageHostSlot::Revoked) {
            self.store.close();
        }
    }
}

/// Release the graph of a window that was destroyed (or never got built);
/// true when no graph window remains. A slot with a page host goes to its
/// retirement (`HostRetirement`); otherwise the Store closes (~200 ms for a
/// Ready graph) after the registry lock is released, so other windows' graph
/// commands do not wait on it (as `load_graph` does for a displaced graph).
/// Cost: one registry write plus the released Store's close on the calling
/// thread.
pub(crate) fn release_window_graph(graphs: &RwLock<GraphRegistry>, window: &str) -> bool {
    let (released, empty) = {
        let mut registry = graphs.write().unwrap();
        let released = registry.remove(window);
        (released, registry.len() == 0)
    };
    drop(released);
    empty
}

/// Windows (`main`, `graph-*`) whose close request has arrived and which have
/// not been destroyed yet: their frontend is flushing, or waiting on a prompt.
///
/// **Question answered.** "May the process exit now, or is another window
/// still saving?" The exit paths (`close_graph_window`, the `Destroyed` hook)
/// both ask [`ClosingWindows::others`]; a window that is not the last one to
/// finish only destroys itself and leaves the exit to the last finisher. A
/// cancelled close leaves its entry behind, which is harmless: that window is
/// still open, so the process rightly stays alive.
#[derive(Default)]
pub(crate) struct ClosingWindows(Mutex<std::collections::HashSet<WindowKey>>);

impl ClosingWindows {
    pub(crate) fn begin(&self, label: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(label.to_string());
    }

    pub(crate) fn end(&self, label: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(label);
    }

    /// How many windows other than `own` are mid-close and still exist.
    pub(crate) fn others(&self, own: &str, exists: impl Fn(&str) -> bool) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|label| label.as_str() != own && exists(label))
            .count()
    }
}

/// Whether a closing window may take the process down with it. The window
/// exits only when it is the last one standing: no other window is still
/// closing (its flush may be running), and no graph is left that another
/// window owns (`own_slot` is whether the closing window holds a graph).
pub(crate) fn may_exit(own_slot: bool, graph_slots: usize, others_closing: usize) -> bool {
    others_closing == 0 && graph_slots <= usize::from(own_slot)
}

/// Whether one graph root contains the other: two Stores and hosts must
/// never hold the same page files.
pub(crate) fn roots_overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

fn still_saving(root: &Path, saving: &Path) -> String {
    format!(
        "graph {} overlaps graph {}, which is still saving its pages; open it again once that graph has closed",
        root.display(),
        saving.display()
    )
}

/// Before an open of `root`: wait, outside the registry lock, up to `bound`
/// for each retiring graph whose root overlaps it (another root: the same
/// root is adopted) to save and close. `waiting` runs once, with that
/// graph's root, when the open has to wait. Past the bound, the refusal
/// `bind` would give.
pub(crate) fn wait_for_retiring_overlaps(
    graphs: &RwLock<GraphRegistry>,
    root: &Path,
    bound: Duration,
    waiting: impl FnOnce(&Path),
) -> Result<(), String> {
    let retirement = graphs.read().unwrap().retirement.clone();
    retirement
        .wait_overlapping(root, bound, waiting)
        .map_err(|saving| still_saving(root, &saving))
}

#[derive(Default)]
pub(crate) struct GraphRegistry {
    by_window: HashMap<WindowKey, Arc<GraphSlot>>,
    by_root: HashMap<PathBuf, WindowKey>,
    /// Owns every released slot that holds a page host until its Store has
    /// closed. A slot leaves the registry and enters it under the registry
    /// lock, so no opener sees its root unowned meanwhile (plan v3 §3, S2).
    pub(crate) retirement: crate::host_retirement::HostRetirement,
}

impl GraphRegistry {
    pub(crate) fn slot(&self, window: &str) -> Option<Arc<GraphSlot>> {
        self.by_window.get(window).cloned()
    }

    pub(crate) fn owner(&self, root: &Path) -> Option<WindowKey> {
        self.by_root.get(root).cloned()
    }

    pub(crate) fn entries(&self) -> Vec<(WindowKey, Arc<GraphSlot>)> {
        self.by_window
            .iter()
            .map(|(window, slot)| (window.clone(), slot.clone()))
            .collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.by_window.len()
    }

    /// Bind `slot` to `window`. A replaced binding is revoked and retired,
    /// or returned when it holds no page host, so the caller drops it after
    /// releasing the registry lock: closing a Ready Store takes ~200 ms
    /// (master abf7af831884).
    pub(crate) fn bind(
        &mut self,
        window: WindowKey,
        slot: Arc<GraphSlot>,
    ) -> Result<Option<Arc<GraphSlot>>, String> {
        for (root, owner) in &self.by_root {
            if owner != &window && roots_overlap(root, &slot.root_key) {
                return Err(format!(
                    "graph {} overlaps graph {} already owned by window {owner}",
                    slot.root_key.display(),
                    root.display()
                ));
            }
        }
        // A retiring host still owns its root (plan v3 §3, S2); the open
        // waited for it first (`wait_for_retiring_overlaps`).
        if let Some(saving) = self.retirement.overlapping(&slot.root_key, true) {
            return Err(still_saving(&slot.root_key, &saving));
        }
        let displaced = self.by_window.insert(window.clone(), slot.clone());
        if let Some(old) = &displaced {
            // A graph switch revokes the old binding. Same-root scans keep the
            // slot and never pass through the registry.
            if old.binding_generation != slot.binding_generation || old.root_key != slot.root_key {
                old.cancel_background();
            }
            self.by_root.remove(&old.root_key);
        }
        self.by_root.insert(slot.root_key.clone(), window);
        Ok(displaced
            .filter(|old| !Arc::ptr_eq(old, &slot))
            .and_then(|old| self.retirement.retire(old)))
    }

    /// Release `window`'s binding only if it is still the one an open with
    /// `binding_generation` created: the window closed while that open ran
    /// off the UI thread, after `WindowEvent::Destroyed` had already cleaned
    /// up (master abf7af831884). A later open's binding is left alone.
    pub(crate) fn release_binding(
        &mut self,
        window: &str,
        binding_generation: u64,
    ) -> Option<Arc<GraphSlot>> {
        let owns = self
            .by_window
            .get(window)
            .is_some_and(|slot| slot.binding_generation == binding_generation);
        if owns {
            self.remove(window)
        } else {
            None
        }
    }

    /// Unbind `window`. Its slot is retired, or returned when it holds no
    /// page host, for the caller to drop outside the registry lock.
    pub(crate) fn remove(&mut self, window: &str) -> Option<Arc<GraphSlot>> {
        let slot = self.by_window.remove(window)?;
        slot.cancel_background();
        self.by_root.remove(&slot.root_key);
        self.retirement.retire(slot)
    }
}

/// Owns each native show from request through one frozen graph selection.
/// Beginning revokes the older lease; completion can consume only the current
/// pending request. Generation checks also own delayed focus callbacks. O(1),
/// memory only; a cold launch remains pending until a graph is published.
#[derive(Default)]
pub(crate) struct CaptureShow {
    generation: u64,
    pending: bool,
    binding: Option<CaptureGraphBinding>,
}

impl CaptureShow {
    fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.pending = true;
        self.binding = None;
        self.generation
    }

    fn complete(&mut self, generation: u64, binding: CaptureGraphBinding) -> bool {
        if self.generation != generation || !self.pending {
            return false;
        }
        self.pending = false;
        self.binding = Some(binding);
        true
    }
}

pub(crate) struct AppState {
    pub(crate) graphs: RwLock<GraphRegistry>,
    // Serializes open/switch/window-create decisions. Existing commands never
    // take this lock, so a slow graph open cannot stall another graph's editor.
    pub(crate) graph_load: Mutex<()>,
    pub(crate) last_focused: Mutex<Option<WindowKey>>,
    pub(crate) capture_graph: Mutex<CaptureShow>,
    #[cfg(desktop)]
    pub(crate) next_window: AtomicU64,
}

impl AppState {
    /// Record the graph window that commands such as quick capture should use.
    ///
    /// Explicit graph activation must update this state synchronously: some
    /// headless window managers, and occasionally desktop focus hand-offs, do
    /// not deliver a later `WindowEvent::Focused` even when `set_focus` was
    /// requested successfully.
    pub(crate) fn note_focused(&self, label: &str) -> bool {
        let mut last = self.last_focused.lock().unwrap();
        if last.as_deref() == Some(label) {
            false
        } else {
            *last = Some(label.to_string());
            true
        }
    }

    /// Atomically publish the graph snapshot selected for the next Quick
    /// Capture show. The capture WebView must present this exact generation on
    /// every graph-scoped invoke; a later show, graph switch, or close makes
    /// older requests stale rather than letting them read another graph.
    #[cfg(test)]
    pub(crate) fn bind_capture_graph(&self, target: WindowKey, binding_generation: u64) {
        let generation = self.begin_capture_show();
        assert!(self.complete_capture_show(generation, target, binding_generation));
    }

    pub(crate) fn begin_capture_show(&self) -> u64 {
        self.capture_graph.lock().unwrap().begin()
    }

    pub(crate) fn pending_capture_show(&self) -> Option<u64> {
        let show = self.capture_graph.lock().unwrap();
        show.pending.then_some(show.generation)
    }

    pub(crate) fn complete_capture_show(
        &self,
        generation: u64,
        target: WindowKey,
        binding_generation: u64,
    ) -> bool {
        self.capture_graph.lock().unwrap().complete(
            generation,
            CaptureGraphBinding {
                target,
                binding_generation,
            },
        )
    }

    pub(crate) fn capture_show_is_current(&self, generation: u64) -> bool {
        let show = self.capture_graph.lock().unwrap();
        show.generation == generation && show.binding.is_some()
    }

    pub(crate) fn bound_capture_show(&self) -> Option<u64> {
        let show = self.capture_graph.lock().unwrap();
        show.binding.as_ref().map(|_| show.generation)
    }

    pub(crate) fn capture_graph_binding(&self) -> Option<CaptureGraphBinding> {
        self.capture_graph.lock().unwrap().binding.clone()
    }
}

pub(crate) struct GraphContext<'a, R: Runtime = tauri::Wry> {
    pub(crate) state: State<'a, AppState>,
    pub(crate) window: WebviewWindow<R>,
    binding_generation: Option<u64>,
}

impl<'r, 'de: 'r, R: Runtime> CommandArg<'de, R> for GraphContext<'r, R> {
    fn from_command(command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
        let binding_generation = match command.message.payload() {
            InvokeBody::Json(value) => value
                .get("bindingGeneration")
                .or_else(|| value.get("binding_generation"))
                .and_then(|v| v.as_u64()),
            InvokeBody::Raw(_) => None,
        };
        let state: State<'r, AppState> = command
            .message
            .state_ref()
            .try_get()
            .ok_or_else(|| InvokeError::from("AppState is not managed"))?;
        let window = WebviewWindow::<R>::from_command(command)?;
        Ok(Self {
            state,
            window,
            binding_generation,
        })
    }
}

pub(crate) fn canonical_graph_root(path: &str) -> Result<PathBuf, String> {
    Store::canonical_root(Path::new(path)).map_err(|error| error.to_string())
}

pub(crate) fn slot_for_window(state: &AppState, window: &str) -> Result<Arc<GraphSlot>, String> {
    state
        .graphs
        .read()
        .unwrap()
        .slot(window)
        .ok_or_else(|| format!("no graph loaded for window {window}"))
}

pub(crate) fn slot_for_context(ctx: &GraphContext<'_>) -> Result<Arc<GraphSlot>, String> {
    slot_for_bound_window(&ctx.state, ctx.window.label(), ctx.binding_generation)
}

/// R3 / I-21: run a command's blocking work (the store writer, an fsync, a
/// directory sync) on the blocking pool and await it, so the main thread keeps
/// painting and serving IPC. A synchronous Tauri command runs on the main
/// thread and every other synchronous command queues behind it. Ordering: a
/// synchronous command was ordered by the main thread; an async one is not, so
/// the frontend issues these commands through its ordered lane (`ORDERED_COMMANDS`
/// in src/orderedWrites.ts) and the shared state each touches keeps its own lock
/// (store writer, SETTINGS_LOCK, DRAFTS_LOCK, NOTICES_LOCK, WORKSPACES_LOCK).
/// Exemplar: commands.rs `save_pages`. A panicking job returns its join error.
pub(crate) async fn off_ui<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

/// Resolve a normal graph-window command. Quick Capture intentionally has no
/// graph slot, so this path cannot be used to grant it any GraphContext command
/// (including save, delete, trash, or other mutations).
pub(crate) fn slot_for_bound_window(
    state: &AppState,
    window: &str,
    binding_generation: Option<u64>,
) -> Result<Arc<GraphSlot>, String> {
    let slot = slot_for_window(state, window)?;
    let generation = binding_generation.ok_or("missing-graph-binding")?;
    if generation != slot.binding_generation {
        return Err(STALE_BINDING.into());
    }
    Ok(slot)
}

/// Resolve the only graph capability granted to the capture WebView: a bounded
/// page/tag quick-switch query. This is deliberately not a GraphContext route;
/// capture retains no generic read or write access to the selected graph.
pub(crate) fn capture_quick_switch_slot(
    state: &AppState,
    caller: &str,
    binding_generation: Option<u64>,
) -> Result<Arc<GraphSlot>, String> {
    if caller != "capture" {
        return Err("capture quick switch is only available to quick capture".into());
    }
    let capture = state
        .capture_graph_binding()
        .ok_or("no graph bound for quick capture")?;
    let generation = binding_generation.ok_or("missing-graph-binding")?;
    if generation != capture.binding_generation {
        return Err(STALE_BINDING.into());
    }
    let slot = slot_for_window(state, &capture.target)?;
    if slot.binding_generation != capture.binding_generation {
        return Err(STALE_BINDING.into());
    }
    Ok(slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quit race: with several windows closing at once, an early finisher
    /// must not exit the process while another window is still flushing.
    #[test]
    fn the_process_exits_only_when_the_last_closing_window_finishes() {
        // Sole window, sole graph: exits (the ordinary quit).
        assert!(may_exit(true, 1, 0));
        // Slotless window and no graph at all: exits.
        assert!(may_exit(false, 0, 0));
        // Another graph window is open and not closing: only this one closes.
        assert!(!may_exit(true, 2, 0));
        // A slotless window must not take a still-open graph down with it.
        assert!(!may_exit(false, 1, 0));
        // The race itself: this window is the last graph, but another window
        // is still flushing, so it must wait for that window to finish.
        assert!(!may_exit(true, 1, 1));
        assert!(!may_exit(false, 0, 1));
    }

    #[test]
    fn closing_windows_ignore_themselves_and_windows_that_are_gone() {
        let closing = ClosingWindows::default();
        closing.begin("main");
        closing.begin("graph-2");
        assert_eq!(closing.others("main", |_| true), 1);
        assert_eq!(closing.others("graph-9", |_| true), 2);
        // A destroyed window is not a pending flush.
        assert_eq!(closing.others("main", |label| label != "graph-2"), 0);
        closing.end("graph-2");
        assert_eq!(closing.others("main", |_| true), 0);
    }

    /// Both exit paths must consult the in-flight set; a path that decides from
    /// the graph count alone is the original race (tray Quit closes every window
    /// at once, so the first finisher exited under the others' saves).
    #[test]
    fn every_exit_path_waits_for_windows_still_closing() {
        let commands = include_str!("commands.rs");
        let close = &commands[commands.find("fn close_graph_window").unwrap()..];
        let close = &close[..close.find("\n}\n").unwrap()];
        assert!(close.contains("may_exit(") && close.contains("ClosingWindows"));
        let lib = include_str!("lib.rs");
        let destroyed = &lib[lib.find("WindowEvent::Destroyed => {").unwrap()..];
        let destroyed = &destroyed[..destroyed.find("_ => {}").unwrap()];
        assert!(destroyed.contains(".others(") && destroyed.contains("app.exit(0)"));
        assert!(lib.contains("WindowEvent::CloseRequested { .. } => {"));
    }

    fn graph(root: &Path) -> Arc<GraphSlot> {
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        Arc::new(GraphSlot::new(
            Store::open(root, tine_store::OpenOptions::default())
                .unwrap()
                .0,
            root.to_path_buf(),
        ))
    }

    #[test]
    fn explicit_graph_activation_updates_capture_routing_idempotently() {
        let state = AppState {
            graphs: RwLock::new(GraphRegistry::default()),
            graph_load: Mutex::new(()),
            last_focused: Mutex::new(Some("graph-1".into())),
            capture_graph: Mutex::new(Default::default()),
            #[cfg(desktop)]
            next_window: AtomicU64::new(2),
        };

        assert!(state.note_focused("main"));
        assert_eq!(state.last_focused.lock().unwrap().as_deref(), Some("main"));
        assert!(!state.note_focused("main"));
    }

    #[test]
    fn pending_capture_show_is_completed_once_and_newer_show_revokes_it() {
        let mut show = CaptureShow::default();
        let first = show.begin();
        let second = show.begin();
        let binding = CaptureGraphBinding {
            target: "main".into(),
            binding_generation: 17,
        };
        assert!(
            !show.complete(first, binding.clone()),
            "I-20: an older show cannot bind the newer capture request"
        );
        assert!(show.pending);
        assert!(show.complete(second, binding.clone()));
        assert_eq!(show.binding, Some(binding));
        assert!(!show.complete(
            second,
            CaptureGraphBinding {
                target: "other".into(),
                binding_generation: 18
            }
        ));
        assert_eq!(show.binding.as_ref().unwrap().target, "main");
        assert!(!show.pending);
        assert!(show.begin() > second);
        assert!(show.binding.is_none());
    }

    #[test]
    fn capture_binding_retains_the_selected_graph_lease() {
        let state = AppState {
            graphs: RwLock::new(GraphRegistry::default()),
            graph_load: Mutex::new(()),
            last_focused: Mutex::new(Some("main".into())),
            capture_graph: Mutex::new(Default::default()),
            #[cfg(desktop)]
            next_window: AtomicU64::new(2),
        };

        state.bind_capture_graph("main".into(), 17);
        assert_eq!(
            state.capture_graph_binding(),
            Some(CaptureGraphBinding {
                target: "main".into(),
                binding_generation: 17,
            })
        );
        state.bind_capture_graph("graph-1".into(), 18);
        assert_eq!(
            state.capture_graph_binding(),
            Some(CaptureGraphBinding {
                target: "graph-1".into(),
                binding_generation: 18,
            })
        );
    }

    #[test]
    fn same_root_refresh_preserves_frontend_binding_lease() {
        let base = std::env::temp_dir().join(format!("tine-slot-refresh-{}", std::process::id()));
        let old = graph(&base);
        old.warm_done
            .store(true, std::sync::atomic::Ordering::Release);
        old.warm_generation
            .store(7, std::sync::atomic::Ordering::Release);

        let binding = old.binding_generation;
        old.store.refresh(tine_store::Depth::Stamps).unwrap();

        assert_eq!(old.binding_generation, binding);
        assert_eq!(old.root_key, base);
        assert!(old.warm_done.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(
            old.warm_generation
                .load(std::sync::atomic::Ordering::Acquire),
            7
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn old_slot_can_commit_during_same_root_refresh() {
        let base = std::env::temp_dir().join(format!(
            "tine-slot-overlap-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let old = graph(&base);
        let mut registry = GraphRegistry::default();
        registry.bind("main".into(), Arc::clone(&old)).unwrap();
        old.store.refresh(tine_store::Depth::Stamps).unwrap();
        assert_eq!(
            registry.slot("main").unwrap().binding_generation,
            old.binding_generation
        );
        let id = old
            .store
            .file_id(tine_store::Area::Pages, "DuringRefresh.md")
            .unwrap();
        let mut tx = old
            .store
            .transaction(Some(tine_store::EditKind::CreatePage));
        tx.create(&id, tine_store::Content::Bytes(b"- retained\n".to_vec()));
        assert!(matches!(
            tx.commit(),
            tine_store::TxOutcome::Committed { .. }
        ));
        assert_eq!(
            std::fs::read(base.join("pages/DuringRefresh.md")).unwrap(),
            b"- retained\n"
        );
        drop(registry);
        drop(old);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn registry_keeps_window_and_root_indices_in_sync() {
        let base = std::env::temp_dir().join(format!("tine-registry-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        let mut registry = GraphRegistry::default();
        let old = graph(&a);
        registry.bind("main".into(), old.clone()).unwrap();
        assert_eq!(registry.owner(&a).as_deref(), Some("main"));
        registry.bind("main".into(), graph(&b)).unwrap();
        assert!(old
            .background_cancelled
            .load(std::sync::atomic::Ordering::Acquire));
        assert!(registry.owner(&a).is_none());
        assert_eq!(registry.owner(&b).as_deref(), Some("main"));
        let current = registry.slot("main").unwrap();
        registry.remove("main");
        assert!(current
            .background_cancelled
            .load(std::sync::atomic::Ordering::Acquire));
        assert!(registry.owner(&b).is_none());
        assert_eq!(registry.len(), 0);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_graph_switch_hands_the_displaced_slot_back_to_drop_outside_the_lock() {
        // Closing a Ready store takes ~200 ms (measured on a copy of the
        // anonymized graph); dropping it inside `bind` held the registry write
        // lock, and every graph command, for that long (master abf7af831884).
        let base =
            std::env::temp_dir().join(format!("tine-registry-displaced-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        let mut registry = GraphRegistry::default();
        let old = graph(&a);
        assert!(registry.bind("main".into(), old.clone()).unwrap().is_none());
        let displaced = registry.bind("main".into(), graph(&b)).unwrap();
        assert!(displaced.is_some_and(|slot| Arc::ptr_eq(&slot, &old)));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_window_closed_during_an_open_releases_only_that_opens_binding() {
        let base =
            std::env::temp_dir().join(format!("tine-registry-closed-{}", std::process::id()));
        let a = base.join("a");
        let mut registry = GraphRegistry::default();
        let slot = graph(&a);
        registry.bind("graph-3".into(), slot.clone()).unwrap();
        assert!(registry
            .release_binding("graph-3", slot.binding_generation + 1)
            .is_none());
        assert_eq!(registry.owner(&a).as_deref(), Some("graph-3"));
        let released = registry.release_binding("graph-3", slot.binding_generation);
        assert!(released.is_some_and(|released| Arc::ptr_eq(&released, &slot)));
        assert!(registry.owner(&a).is_none());
        assert!(slot
            .background_cancelled
            .load(std::sync::atomic::Ordering::Acquire));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_destroyed_window_closes_its_graph_after_releasing_the_registry_lock() {
        // Closing a Ready Store takes ~200 ms; under the registry write lock
        // that stalled every other window's graph commands.
        let base =
            std::env::temp_dir().join(format!("tine-registry-destroyed-{}", std::process::id()));
        let root = base.join("a");
        let graphs = Arc::new(RwLock::new(GraphRegistry::default()));
        graphs
            .write()
            .unwrap()
            .bind("graph-2".into(), graph(&root))
            .unwrap();
        let observed: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
        {
            let (graphs, observed, root) = (graphs.clone(), observed.clone(), root.clone());
            *SLOT_CLOSE_PROBE.lock().unwrap() = Some(Box::new(move |closing: &Path| {
                if closing == root {
                    *observed.lock().unwrap() = Some(graphs.try_write().is_ok());
                }
            }));
        }
        let empty = release_window_graph(&graphs, "graph-2");
        *SLOT_CLOSE_PROBE.lock().unwrap() = None;
        assert!(empty);
        assert_eq!(
            *observed.lock().unwrap(),
            Some(true),
            "the destroyed window's Store must close with the registry lock released"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    /// Registry guards taken in the head of an `if let` / `while let` /
    /// `match` / `for` live for the whole body (edition 2021 temporary
    /// lifetime). Returns `file:line` for each such head.
    fn registry_guards_in_branch_heads(file: &str, source: &str) -> Vec<String> {
        let code: String = source
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("//") {
                    ""
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let bytes = code.as_bytes();
        let mut found = Vec::new();
        for keyword in ["if let ", "while let ", "match ", "for "] {
            for (start, _) in code.match_indices(keyword) {
                let boundary = start == 0 || !(bytes[start - 1] as char).is_alphanumeric();
                if !boundary || (start > 0 && bytes[start - 1] == b'_') {
                    continue;
                }
                let (mut depth, mut end) = (0i32, None);
                for (offset, ch) in code[start..].char_indices() {
                    match ch {
                        '(' | '[' => depth += 1,
                        ')' | ']' => {
                            depth -= 1;
                            if depth < 0 {
                                break; // inside a string or argument list
                            }
                        }
                        '{' if depth == 0 => {
                            end = Some(start + offset);
                            break;
                        }
                        ';' if depth == 0 => break,
                        _ => {}
                    }
                }
                let Some(end) = end else { continue };
                let head: String = code[start..end].split_whitespace().collect();
                if head.contains("graphs.read()") || head.contains("graphs.write()") {
                    let line = code[..start].matches('\n').count() + 1;
                    found.push(format!("{file}:{line}"));
                }
            }
        }
        found
    }

    #[test]
    fn registry_guards_never_live_across_a_branch_body() {
        // R2 / I-21: a registry guard in a branch head stays held while the
        // body re-reads the registry, activates windows, fsyncs settings or
        // stops watchers. A new `read()` then blocks behind any queued writer
        // (bind, the `Destroyed` handler) that itself waits on the held guard:
        // a deadlock that freezes the UI. Snapshot into a `let`, then act.
        // Exemplar: graph.rs `route_bound_root`.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut pending = vec![dir];
        let mut offenders = Vec::new();
        let mut scanned = 0;
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let source = std::fs::read_to_string(&path).unwrap();
                    scanned += 1;
                    offenders.extend(registry_guards_in_branch_heads(
                        &path.display().to_string(),
                        &source,
                    ));
                }
            }
        }
        assert!(scanned > 20, "the scan must cover the crate's sources");
        assert_eq!(
            registry_guards_in_branch_heads(
                "probe",
                concat!(
                    "if let Some(o) = state.graphs.",
                    "read().unwrap().owner(r) {}"
                ),
            ),
            vec!["probe:1".to_string()],
            "the scanner must recognise the forbidden shape"
        );
        assert!(
            offenders.is_empty(),
            "registry guard held across a branch body (snapshot into a `let` first; \
             exemplar graph.rs route_bound_root): {offenders:?}"
        );
    }

    #[test]
    fn window_graphs_are_released_only_through_the_lock_releasing_helper() {
        // A slot removed inline (`graphs.write().unwrap().remove(..)`) closes
        // its Store while the registry lock is held; exemplar
        // state.rs release_window_graph.
        for (file, source) in [
            ("lib.rs", include_str!("lib.rs")),
            ("graph.rs", include_str!("graph.rs")),
            ("commands.rs", include_str!("commands.rs")),
            ("watcher.rs", include_str!("watcher.rs")),
        ] {
            let compact: String = source.split_whitespace().collect();
            assert!(
                !compact.contains(".write().unwrap().remove("),
                "{file} closes a graph under the registry lock; use state::release_window_graph"
            );
        }
    }

    #[test]
    fn registry_rejects_two_windows_for_one_root() {
        let base = std::env::temp_dir().join(format!("tine-registry-dupe-{}", std::process::id()));
        let mut registry = GraphRegistry::default();
        registry.bind("main".into(), graph(&base)).unwrap();
        assert!(registry.bind("graph-1".into(), graph(&base)).is_err());
        assert!(registry.slot("graph-1").is_none());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn registry_rejects_ancestor_and_descendant_graph_roots() {
        let base =
            std::env::temp_dir().join(format!("tine-registry-nested-{}", std::process::id()));
        let parent = base.join("parent");
        let child = parent.join("pages").join("child");
        let sibling = base.join("sibling");

        let mut registry = GraphRegistry::default();
        registry.bind("main".into(), graph(&parent)).unwrap();
        assert!(registry.bind("child".into(), graph(&child)).is_err());
        assert!(registry.bind("sibling".into(), graph(&sibling)).is_ok());

        let mut reverse = GraphRegistry::default();
        reverse.bind("child".into(), graph(&child)).unwrap();
        assert!(reverse.bind("parent".into(), graph(&parent)).is_err());
        let _ = std::fs::remove_dir_all(base);
    }
}
