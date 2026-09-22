use crate::command_error::CommandError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tauri::ipc::{CommandArg, CommandItem, InvokeBody, InvokeError};
use tauri::{Manager, Runtime, State, WebviewWindow};
use tine_core::model::Graph;

pub(crate) type WindowKey = String;
static NEXT_BINDING: AtomicU64 = AtomicU64::new(1);

/// The current graph binding's page-write admission, as the frontend wire
/// record `{ binding_generation }`. Its presence admits page writes; the
/// generation fences a queued mutation against a window that was rebound.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ApplicationPageAdmission {
    pub(crate) binding_generation: u64,
}

/// Read-only graph lease used by the auxiliary Quick Capture WebView. Capture
/// deliberately does not own a graph slot: the registry permits one writable
/// window per graph root, while this surface only needs the selected graph's
/// query/read commands before it hands writes back to the owning window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureGraphBinding {
    pub(crate) target: WindowKey,
    pub(crate) binding_generation: u64,
}

#[derive(Debug)]
pub(crate) enum AssetStreamError {
    InvalidAsset,
}

pub(crate) struct GraphSlot {
    graph: Arc<Graph>,
    /// The configuration snapshot `load_graph` hands the frontend. Replaceable
    /// so a settings change can update it in place: a config refresh publishes
    /// a whole replacement slot after a reopen, and without this a window would
    /// report the settings the graph opened with the next time it loads the
    /// same root.
    graph_meta: RwLock<tine_core::model::GraphMeta>,
    pub(crate) root_key: PathBuf,
    /// Unique lease for this exact window→graph binding. Frontend mutations carry
    /// it so an IPC queued before an in-place graph switch cannot execute against
    /// the replacement graph after the window label is rebound.
    pub(crate) binding_generation: u64,
    pub(crate) warm_done: AtomicBool,
    pub(crate) warm_generation: AtomicU64,
    /// Revoked as soon as this exact window→graph binding is replaced/removed.
    /// Detached warm/backup workers check it before and during graph-sized work.
    pub(crate) background_cancelled: Arc<AtomicBool>,
}

impl GraphSlot {
    pub(crate) fn new(graph: Graph, root_key: PathBuf) -> Self {
        let graph_meta = graph.meta();
        Self {
            graph: Arc::new(graph),
            graph_meta: RwLock::new(graph_meta),
            root_key,
            binding_generation: NEXT_BINDING.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            warm_done: AtomicBool::new(false),
            warm_generation: AtomicU64::new(0),
            background_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// This binding's graph. The slot owns exactly one graph for its whole
    /// life, so the handle is infallible; callers that outlive the command
    /// (watchers, warm and backup workers) keep the returned `Arc`.
    pub(crate) fn graph(&self) -> Arc<Graph> {
        Arc::clone(&self.graph)
    }

    /// Report the selected save route for this exact graph binding.
    pub(crate) fn application_page_admission(&self) -> ApplicationPageAdmission {
        ApplicationPageAdmission {
            binding_generation: self.binding_generation,
        }
    }

    /// Resolve one range-streamed asset without borrowing graph-text write
    /// authority. Containment is the one canonical `Graph::stream_asset_path`
    /// implementation.
    pub(crate) fn asset_stream_path(&self, name: &str) -> Result<PathBuf, AssetStreamError> {
        self.graph
            .stream_asset_path(name)
            .map_err(|_| AssetStreamError::InvalidAsset)
    }

    /// Persist a change to `logseq/config.edn`.
    ///
    /// Settings use the short-lived filesystem capability, enforced in
    /// `tine-core` by `Graph::ensure_config_write_target`, without graph-text
    /// authority.
    pub(crate) fn with_config_graph<T>(
        &self,
        f: impl FnOnce(&Graph) -> Result<T, CommandError>,
    ) -> Result<T, CommandError> {
        self.with_filesystem_graph(f)
    }

    /// Move something into (or clear) the recoverable trash.
    ///
    /// Trashing a page, journal or conflict copy is a graph-text deletion and
    /// is admitted inside `tine-core`, at `Graph::admit_graph_text_writer`.
    pub(crate) fn with_trash_graph<T>(
        &self,
        f: impl FnOnce(&Graph) -> Result<T, CommandError>,
    ) -> Result<T, CommandError> {
        self.with_filesystem_graph(f)
    }

    /// Run one point-addressed filesystem/config/asset operation.
    pub(crate) fn with_filesystem_graph<T>(
        &self,
        f: impl FnOnce(&Graph) -> Result<T, CommandError>,
    ) -> Result<T, CommandError> {
        f(self.graph.as_ref())
    }

    pub(crate) fn graph_meta(&self) -> tine_core::model::GraphMeta {
        self.graph_meta.read().unwrap().clone()
    }

    /// Persist a presentation-only setting and update the meta the frontend
    /// reads, keeping this `Graph` and its index. Only for settings nothing in
    /// the core reads: `refresh_graph` retires the projection worker, reopens
    /// the graph and starts the launch check again, which on a first launch
    /// threw away the whole index build for a toggle (GH #543, IT-06).
    pub(crate) fn apply_presentation_setting(
        &self,
        write: impl FnOnce(&Graph) -> std::io::Result<()>,
        update: impl FnOnce(&mut tine_core::model::GraphMeta),
    ) -> Result<(), CommandError> {
        self.with_config_graph(|graph| write(graph).map_err(CommandError::from))?;
        update(&mut self.graph_meta.write().unwrap());
        Ok(())
    }

    /// Re-open the graph object for the same window/root without revoking the
    /// frontend's lease. A binding generation identifies a window -> graph-root
    /// assignment, not the particular in-memory `Graph` instance. Minting a new
    /// generation here made every later command from that window stale after a
    /// config refresh, including autosaves.
    fn refreshed(graph: Graph, old: &GraphSlot) -> Result<Self, CommandError> {
        let graph_meta = graph.meta();
        Ok(Self {
            graph: Arc::new(graph),
            graph_meta: RwLock::new(graph_meta),
            root_key: old.root_key.clone(),
            binding_generation: old.binding_generation,
            warm_done: AtomicBool::new(old.warm_done.load(std::sync::atomic::Ordering::Acquire)),
            warm_generation: AtomicU64::new(
                old.warm_generation
                    .load(std::sync::atomic::Ordering::Acquire),
            ),
            background_cancelled: Arc::clone(&old.background_cancelled),
        })
    }
}

#[derive(Default)]
pub(crate) struct GraphRegistry {
    by_window: HashMap<WindowKey, Arc<GraphSlot>>,
    by_root: HashMap<PathBuf, WindowKey>,
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

    pub(crate) fn bind(
        &mut self,
        window: WindowKey,
        slot: Arc<GraphSlot>,
    ) -> Result<(), CommandError> {
        for (root, owner) in &self.by_root {
            if owner != &window
                && (root.starts_with(&slot.root_key) || slot.root_key.starts_with(root))
            {
                return Err(CommandError::prose(format!(
                    "graph {} overlaps graph {} already owned by window {owner}",
                    slot.root_key.display(),
                    root.display()
                )));
            }
        }
        if let Some(old) = self.by_window.insert(window.clone(), slot.clone()) {
            // A same-root refresh replaces only the in-memory Graph object and
            // preserves the frontend binding lease. Let its already-running
            // warm/backup finish; a real graph switch revokes the old source.
            if old.binding_generation != slot.binding_generation || old.root_key != slot.root_key {
                old.background_cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
                old.graph().retire();
            }
            self.by_root.remove(&old.root_key);
        }
        self.by_root.insert(slot.root_key.clone(), window);
        Ok(())
    }

    pub(crate) fn remove(&mut self, window: &str) -> Option<Arc<GraphSlot>> {
        let slot = self.by_window.remove(window)?;
        slot.background_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        slot.graph().retire();
        // This revokes Tauri background work and display reads.
        self.by_root.remove(&slot.root_key);
        Some(slot)
    }
}

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
    /// Sole owner of serialized open/switch/storage-mode transitions and their
    /// typed native operation model.
    pub(crate) storage_supervisor:
        crate::storage_transition_supervisor::StorageTransitionSupervisor,
    pub(crate) watch_ctl: Mutex<Option<Sender<()>>>,
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
    pub(crate) binding_generation: Option<u64>,
}

pub(crate) fn owned_graph_context(
    state: GraphContext<'_>,
) -> Result<(tauri::AppHandle, String, u64), CommandError> {
    let app = state.window.app_handle().clone();
    let label = state.window.label().to_string();
    let binding_generation = state
        .binding_generation
        .ok_or_else(|| CommandError::prose("missing-graph-binding"))?;
    drop(state);
    Ok((app, label, binding_generation))
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

pub(crate) fn canonical_graph_root(path: &str) -> Result<PathBuf, CommandError> {
    // Not `fs::canonicalize` directly: an encrypted-volume driver may refuse to
    // answer it at all, which used to make every graph on that drive
    // unopenable (GH #561).
    let root = tine_core::directory_identity::canonical_existing_path(Path::new(path)).map_err(
        |error| CommandError::coded("couldn't resolve graph path", format!("{path}: {error}")),
    )?;
    if !root.is_dir() {
        return Err(CommandError::prose(format!(
            "graph path is not a folder: {}",
            root.display()
        )));
    }
    Ok(root)
}

pub(crate) fn slot_for_window(
    state: &AppState,
    window: &str,
) -> Result<Arc<GraphSlot>, CommandError> {
    state
        .graphs
        .read()
        .unwrap()
        .slot(window)
        .ok_or_else(|| CommandError::prose(format!("no graph loaded for window {window}")))
}

pub(crate) fn slot_for_context(ctx: &GraphContext<'_>) -> Result<Arc<GraphSlot>, CommandError> {
    slot_for_bound_window(&ctx.state, ctx.window.label(), ctx.binding_generation)
}

/// Resolve a normal graph-window command. Quick Capture intentionally has no
/// graph slot, so this path cannot be used to grant it any GraphContext command
/// (including save, delete, trash, or other mutations).
pub(crate) fn slot_for_bound_window(
    state: &AppState,
    window: &str,
    binding_generation: Option<u64>,
) -> Result<Arc<GraphSlot>, CommandError> {
    let slot = slot_for_window(state, window)?;
    let generation =
        binding_generation.ok_or_else(|| CommandError::prose("missing-graph-binding"))?;
    if generation != slot.binding_generation {
        return Err(CommandError::prose("stale-graph-binding"));
    }
    Ok(slot)
}

/// Answer a display-only read (a listing, aliases, icons, counts) from the
/// window's current graph. A read that was running on a graph the app has
/// since replaced returns no answer rather than parse that retired graph, and
/// is asked again of its replacement (GH #543). After a real graph switch the
/// binding no longer matches and the request ends as stale, as before.
pub(crate) fn display_read<T>(
    state: &AppState,
    window: &str,
    binding_generation: u64,
    read: impl Fn(&Graph) -> T,
) -> Result<T, CommandError> {
    // A refresh retires the old graph before it binds the replacement; the
    // bound limits the wait if that bind never comes.
    for _ in 0..500 {
        let graph = slot_for_bound_window(state, window, Some(binding_generation))?.graph();
        if let Some(answer) = graph.display_read(|| read(&graph)) {
            return Ok(answer);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(CommandError::prose("stale-graph-binding"))
}

/// Resolve the only graph capability granted to the capture WebView: a bounded
/// page/tag quick-switch query. This is deliberately not a GraphContext route;
/// capture retains no generic read or write access to the selected graph.
pub(crate) fn capture_quick_switch_slot(
    state: &AppState,
    caller: &str,
    binding_generation: Option<u64>,
) -> Result<Arc<GraphSlot>, CommandError> {
    if caller != "capture" {
        return Err(CommandError::prose(
            "capture quick switch is only available to quick capture",
        ));
    }
    let capture = state
        .capture_graph_binding()
        .ok_or_else(|| CommandError::prose("no graph bound for quick capture"))?;
    let generation =
        binding_generation.ok_or_else(|| CommandError::prose("missing-graph-binding"))?;
    if generation != capture.binding_generation {
        return Err(CommandError::prose("stale-graph-binding"));
    }
    let slot = slot_for_window(state, &capture.target)?;
    if slot.binding_generation != capture.binding_generation {
        return Err(CommandError::prose("stale-graph-binding"));
    }
    Ok(slot)
}

/// Run one non-graph-semantic filesystem/config/asset operation.
pub(crate) fn with_filesystem_graph<T>(
    ctx: &GraphContext<'_>,
    f: impl FnOnce(&Graph) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    slot_for_context(ctx)?.with_filesystem_graph(f)
}

/// Run a `logseq/config.edn` write. See [`GraphSlot::with_config_graph`].
pub(crate) fn with_config_graph<T>(
    ctx: &GraphContext<'_>,
    f: impl FnOnce(&Graph) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    slot_for_context(ctx)?.with_config_graph(f)
}

/// Run a recoverable-trash write under either authority. See
/// [`GraphSlot::with_trash_graph`] for what it does and does not cover.
pub(crate) fn with_trash_graph<T>(
    ctx: &GraphContext<'_>,
    f: impl FnOnce(&Graph) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    slot_for_context(ctx)?.with_trash_graph(f)
}

/// How a refresh should behave when another operation holds the storage
/// transition lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefreshLaneWait {
    /// A user-initiated settings change: wait for the lane.
    Block,
    /// A watcher cycle: never block. Blocking here would stall the reconcile
    /// loop for *every* graph behind one graph's load or storage promotion, so
    /// a busy lane reports `Deferred` and the next cycle tries again -- the
    /// on-disk configuration is still there, so nothing is lost by waiting.
    TryOnce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefreshOutcome {
    Refreshed,
    Deferred,
}

pub(crate) fn refresh_graph(ctx: &GraphContext<'_>) -> Result<(), CommandError> {
    let label = ctx.window.label().to_string();
    refresh_graph_for_label(
        &ctx.state,
        ctx.window.app_handle(),
        &label,
        RefreshLaneWait::Block,
    )
    .map(|_| ())
}

/// Re-read configuration for one window's graph without a `GraphContext`.
///
/// The watcher has a window label and an `AppHandle` and no command context, so
/// this is the shared body; `refresh_graph` is the blocking command-side entry.
pub(crate) fn refresh_graph_for_label(
    state: &AppState,
    app: &tauri::AppHandle,
    label: &str,
    wait: RefreshLaneWait,
) -> Result<RefreshOutcome, CommandError> {
    let label = label.to_string();
    // Refresh may migrate graph files before publishing its replacement slot.
    // Serialize the whole operation with graph loads.
    let root_hint = slot_for_window(state, &label)?.root_key.clone();
    let transition_gate = state.storage_supervisor.transition_lane(&root_hint);
    let _transition = match wait {
        RefreshLaneWait::Block => transition_gate.lock().unwrap(),
        RefreshLaneWait::TryOnce => match transition_gate.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(RefreshOutcome::Deferred),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        },
    };
    let old = slot_for_window(state, &label)?;
    if old.root_key != root_hint {
        return Err(CommandError::prose(
            "graph changed while refresh waited for its transition lane",
        ));
    }
    let approved = crate::settings::approved_external_assets(app, &old.root_key);
    let services = crate::graph::direct_files_service_paths(app, &old.root_key);
    let replacement = Arc::new(reopen_legacy_for_refresh(
        &old,
        approved.as_deref(),
        services,
    )?);
    // The reopened graph starts with a cold parsed cache, and the projection
    // attached above takes its full payload from the warm — exactly as the
    // open path does (`publish_prepared_direct_files`).
    let warm_generation = crate::graph::begin_warm_cache(&replacement);
    state
        .graphs
        .write()
        .unwrap()
        .bind(label.clone(), Arc::clone(&replacement))?;
    crate::graph::warm_cache_async(app.clone(), label, replacement, warm_generation)?;
    poke_watcher(state);
    Ok(RefreshOutcome::Refreshed)
}

/// The app-handle-free body of a legacy (Direct Files) configuration refresh:
/// retire the old graph's projection worker, reopen the root, attach the
/// Direct Files services, and build the replacement slot. The caller binds
/// the slot and starts the warm. Kept separate so the invariant — a refreshed
/// graph carries the same services as an opened one — is testable without a
/// Tauri app.
pub(crate) fn reopen_legacy_for_refresh(
    old: &GraphSlot,
    approved_assets: Option<&Path>,
    services: crate::graph::DirectFilesServicePaths,
) -> Result<GraphSlot, CommandError> {
    let old_graph = old.graph();
    old_graph.retire();
    // The replacement attaches a projection at the SAME path; the old worker
    // must have released the writer lease first or the new one races it.
    if !old_graph.detach_direct_projection(Duration::from_secs(15)) {
        crate::debug::diag(
            "Direct Files projection worker did not stop within 15 s before a refresh; \
             the replacement attach may find its database busy"
                .to_string(),
        );
    }
    drop(old_graph);
    let graph = Graph::open_checked_with_assets(&old.root_key, approved_assets)?;
    // Concord invariant 4: a refresh re-reads configuration, it does not rewrite
    // the tree. Journal filename repairs are proposed and applied explicitly
    // (`apply_journal_filename_migrations`) — a settings change must not rename
    // the user's files as a side effect. (This site did not even take the
    // pre-migration snapshot the open path used to.)
    crate::graph::attach_direct_files_services(&graph, services);
    GraphSlot::refreshed(graph, old)
}

pub(crate) fn poke_watcher(state: &AppState) {
    if let Some(tx) = state.watch_ctl.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::rust_module_source;
    use std::time::Instant;

    fn graph(root: &Path) -> Arc<GraphSlot> {
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        Arc::new(GraphSlot::new(Graph::open(root), root.to_path_buf()))
    }

    /// Rows of the OG query `[[Alpha]]`, straight from the graph's index.
    fn alpha_ref_count(graph: &Graph) -> Result<usize, tine_core::query::QueryExecutionError> {
        let registry = tine_core::query::registry::Registry::from_snapshot(
            &tine_core::query::ir::RegistrySnapshot {
                rows: Vec::new(),
                generation: 0,
            },
        );
        let (query, _view) = tine_core::query::parse_query_input(
            "[[Alpha]]",
            tine_core::query::QueryInput::Og,
            tine_core::date::JournalDate::today(),
            &registry,
        );
        let result = tine_core::query::run_query_result_ir(
            graph,
            &query,
            &tine_core::query::ir::ViewSettings::default(),
            tine_core::query::ir::Bounds::unbounded(),
            &tine_core::query::ir::ExecutionContext::none(),
        )?;
        Ok(match result.rows {
            tine_core::query::ir::QueryRows::Page { pages } => pages.len(),
            tine_core::query::ir::QueryRows::Block { groups } => {
                groups.iter().map(|group| group.blocks.len()).sum()
            }
        })
    }
    fn alpha_ref_count_when_ready(
        graph: &Graph,
    ) -> Result<usize, tine_core::query::QueryExecutionError> {
        let started = Instant::now();
        loop {
            match alpha_ref_count(graph) {
                Err(tine_core::query::QueryExecutionError::NotReady(_)) => {
                    assert!(
                        started.elapsed() < Duration::from_secs(30),
                        "the projection never became ready"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                other => return other,
            }
        }
    }

    /// GH #543 (indexing audit IT-06): a presentation-only setting keeps the
    /// Graph, its index and its launch check. Toggling one used to refresh the
    /// graph: the projection worker was retired and the graph reopened, and on
    /// a first launch -- when dismissing the Guide toast is most likely -- the
    /// whole index build restarted from nothing.
    #[test]
    fn a_presentation_setting_keeps_the_graph_and_its_index() {
        let root = std::env::temp_dir().join(format!(
            "tine-presentation-setting-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["pages", "journals", "logseq"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("logseq/config.edn"), "{}\n").unwrap();
        std::fs::write(root.join("pages/Alpha.md"), "- alpha\n").unwrap();
        std::fs::write(root.join("pages/Beta.md"), "- see [[Alpha]]\n").unwrap();
        let graph = Graph::open_checked_with_assets(&root, None).unwrap();
        crate::graph::attach_direct_files_services(
            &graph,
            crate::graph::DirectFilesServicePaths {
                projection: Ok(root.join("private/projection.sqlite")),
                concord_ledger: None,
            },
        );
        graph.warm_cache();
        let slot = GraphSlot::new(graph, root.clone());
        let before = slot.graph();
        let answered = alpha_ref_count_when_ready(&before).expect("queries answer before");
        assert!(answered > 0, "the fixture has a referring block");
        assert!(!slot.graph_meta().show_brackets || !slot.graph_meta().guide_announced);

        slot.apply_presentation_setting(
            |graph| graph.set_show_brackets(true),
            |meta| meta.show_brackets = true,
        )
        .unwrap();
        slot.apply_presentation_setting(
            |graph| graph.set_guide_announced(true),
            |meta| meta.guide_announced = true,
        )
        .unwrap();

        assert!(
            Arc::ptr_eq(&before, &slot.graph()),
            "the graph was replaced"
        );
        assert!(slot.graph_meta().show_brackets && slot.graph_meta().guide_announced);
        // Persisted: the next open reads both.
        let reopened = Graph::open(&root).meta();
        assert!(reopened.show_brackets && reopened.guide_announced);
        // The index was never detached: the same query answers at once, with
        // no second launch check (a refresh would need one before answering).
        assert_eq!(alpha_ref_count(&before).ok(), Some(answered));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// GH draft "Query Engine" (2026-09-11): dismissing the Guide toast made
    /// every query `ProjectionUnavailable` until the next graph open, because
    /// `set_guide_announced` → `refresh_graph` rebuilt the `Graph` without the
    /// Direct Files projection the open path attaches. Same shape for every
    /// settings command that refreshes, restore-from-backup, and an external
    /// `config.edn` rewrite. The refresh body must attach what the open attaches.
    #[test]
    fn a_config_refresh_keeps_queries_answering() {
        let root = std::env::temp_dir().join(format!(
            "tine-refresh-keeps-projection-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["pages", "journals", "logseq"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("logseq/config.edn"), "{}\n").unwrap();
        std::fs::write(root.join("pages/Alpha.md"), "- alpha root\n").unwrap();
        std::fs::write(
            root.join("pages/Beta.md"),
            "- refers to [[Alpha]]\n- and [[Alpha]] again\n",
        )
        .unwrap();
        let projection = root.join("private/projection.sqlite");
        let services = || crate::graph::DirectFilesServicePaths {
            projection: Ok(projection.clone()),
            concord_ledger: None,
        };

        let graph = Graph::open_checked_with_assets(&root, None).unwrap();
        crate::graph::attach_direct_files_services(&graph, services());
        graph.warm_cache();
        let old = Arc::new(GraphSlot::new(graph, root.clone()));
        let before = alpha_ref_count_when_ready(&old.graph()).expect("queries answer before");
        assert!(before > 0, "the fixture has referring blocks");

        // What a refreshing settings command (`set_preferred_format`,
        // `set_journal_title_format`, `set_default_home`) does: a config write,
        // then a refresh.
        old.graph().set_guide_announced(true).unwrap();
        let replacement = reopen_legacy_for_refresh(&old, None, services()).unwrap();
        // The open path warms through `warm_cache_async`; the refresh core hands
        // that to its caller, so warm here exactly as the caller would.
        let reopened = replacement.graph();
        reopened.warm_cache();
        let after = alpha_ref_count_when_ready(&reopened);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            after.ok(),
            Some(before),
            "after refresh the same query must answer"
        );
    }

    #[test]
    fn owned_command_context_drops_borrowed_tauri_state_after_capturing_exact_binding() {
        let source = include_str!("state.rs");
        let start = source
            .find("pub(crate) fn owned_graph_context(")
            .expect("owned graph context helper");
        let tail = &source[start..];
        let end = tail
            .find("\nimpl<'r, 'de: 'r, R: Runtime> CommandArg")
            .expect("owned graph context helper boundary");
        let helper = &tail[..end];
        let compact: String = helper.split_whitespace().collect();
        for required in [
            "state.window.app_handle().clone()",
            "state.window.label().to_string()",
            "state.binding_generation",
            "drop(state)",
        ] {
            assert!(
                compact.contains(required),
                "owned command context must retain `{required}` before await"
            );
        }
    }

    #[test]
    fn graph_slots_are_send_sync() {
        let base =
            std::env::temp_dir().join(format!("tine-slot-authority-{}", uuid::Uuid::new_v4()));
        let _slot = graph(&base);

        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GraphSlot>();

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn explicit_graph_activation_updates_capture_routing_idempotently() {
        let state = AppState {
            graphs: RwLock::new(GraphRegistry::default()),
            storage_supervisor:
                crate::storage_transition_supervisor::StorageTransitionSupervisor::default(),
            watch_ctl: Mutex::new(None),
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
        assert!(show.binding.is_none());
        let second = show.begin();
        let binding = CaptureGraphBinding {
            target: "main".into(),
            binding_generation: 17,
        };
        assert!(!show.complete(first, binding.clone()));
        assert!(show.pending);
        assert!(show.complete(second, binding.clone()));
        assert_eq!(show.binding, Some(binding));
        // An unrelated graph publication cannot retarget an already shown lease.
        assert!(!show.complete(
            second,
            CaptureGraphBinding {
                target: "other".into(),
                binding_generation: 18,
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
            storage_supervisor:
                crate::storage_transition_supervisor::StorageTransitionSupervisor::default(),
            watch_ctl: Mutex::new(None),
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

        let replacement = GraphSlot::refreshed(Graph::open(&base), &old).unwrap();

        assert_eq!(replacement.binding_generation, old.binding_generation);
        assert_eq!(replacement.root_key, old.root_key);
        assert!(replacement
            .warm_done
            .load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(
            replacement
                .warm_generation
                .load(std::sync::atomic::Ordering::Acquire),
            7
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_display_read_on_a_retired_graph_is_answered_by_its_replacement() {
        let base = std::env::temp_dir().join(format!("tine-retired-read-{}", uuid::Uuid::new_v4()));
        let state = Arc::new(AppState {
            graphs: RwLock::new(GraphRegistry::default()),
            storage_supervisor:
                crate::storage_transition_supervisor::StorageTransitionSupervisor::default(),
            watch_ctl: Mutex::new(None),
            last_focused: Mutex::new(Some("main".into())),
            capture_graph: Mutex::new(Default::default()),
            #[cfg(desktop)]
            next_window: AtomicU64::new(2),
        });
        let old = graph(&base);
        std::fs::write(base.join("pages/Alpha.md"), "- alpha\n").unwrap();
        let generation = old.binding_generation;
        state
            .graphs
            .write()
            .unwrap()
            .bind("main".into(), Arc::clone(&old))
            .unwrap();
        // A refresh retires the old graph and binds its replacement shortly after.
        old.graph().retire();
        let binder = std::thread::spawn({
            let state = Arc::clone(&state);
            let old = Arc::clone(&old);
            move || {
                std::thread::sleep(Duration::from_millis(200));
                let replacement = Graph::open_checked_with_assets(&old.root_key, None).unwrap();
                let slot = Arc::new(GraphSlot::refreshed(replacement, &old).unwrap());
                state
                    .graphs
                    .write()
                    .unwrap()
                    .bind("main".into(), slot)
                    .unwrap();
            }
        });
        let started = std::time::Instant::now();
        let names = display_read(&state, "main", generation, |graph| {
            graph
                .list_pages()
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>()
        })
        .unwrap();
        binder.join().unwrap();
        assert_eq!(names, vec!["Alpha".to_owned()]);
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "the retired graph must not answer by parsing itself; the replacement answers"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn queued_owned_command_generation_is_rejected_after_graph_replacement() {
        let base = std::env::temp_dir().join(format!("tine-slot-stale-{}", uuid::Uuid::new_v4()));
        let old_root = base.join("old");
        let new_root = base.join("new");
        let state = AppState {
            graphs: RwLock::new(GraphRegistry::default()),
            storage_supervisor:
                crate::storage_transition_supervisor::StorageTransitionSupervisor::default(),
            watch_ctl: Mutex::new(None),
            last_focused: Mutex::new(Some("main".into())),
            capture_graph: Mutex::new(Default::default()),
            #[cfg(desktop)]
            next_window: AtomicU64::new(2),
        };
        let old = graph(&old_root);
        let captured_generation = old.binding_generation;
        state
            .graphs
            .write()
            .unwrap()
            .bind("main".into(), old)
            .unwrap();
        let replacement = graph(&new_root);
        assert_ne!(replacement.binding_generation, captured_generation);
        state
            .graphs
            .write()
            .unwrap()
            .bind("main".into(), replacement)
            .unwrap();

        assert_eq!(
            slot_for_bound_window(&state, "main", Some(captured_generation))
                .err()
                .unwrap()
                .to_string(),
            "stale-graph-binding"
        );
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
    fn unpublished_candidate_has_no_registry_or_watcher_ownership() {
        let base = std::env::temp_dir().join(format!(
            "tine-unpublished-candidate-{}",
            uuid::Uuid::new_v4()
        ));
        let serving_root = base.join("serving");
        let candidate_root = base.join("candidate");
        let mut registry = GraphRegistry::default();
        let serving = graph(&serving_root);
        registry.bind("main".into(), Arc::clone(&serving)).unwrap();

        let candidate = graph(&candidate_root);
        assert_eq!(registry.entries().len(), 1);
        assert!(Arc::ptr_eq(&registry.entries()[0].1, &serving));
        assert!(registry.owner(&candidate.root_key).is_none());
        assert_ne!(candidate.binding_generation, serving.binding_generation);

        let watcher = rust_module_source("watcher.rs");
        assert!(watcher.contains("app.state::<AppState>().graphs.read().unwrap().entries()"));
        let _ = std::fs::remove_dir_all(base);
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
