use crate::settings::{settings_path, update_settings};
use crate::state::{AppState, GraphSlot};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{Emitter, Manager, State};
use tine_core::{model::PageKind, Graph};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct GraphChange {
    name: String,
    kind: PageKind,
    removed: bool,
}

#[derive(Default)]
struct Pending {
    paths: HashSet<PathBuf>,
    need_full: bool,
}

impl Pending {
    fn add_event(&mut self, event: notify::Event) {
        if event.need_rescan() {
            self.need_full = true;
            return;
        }
        // Page files take the narrow incremental path. Managed-sync chunks and
        // receipts are also concrete paths, but use their own reconcile lane.
        // Unknown/empty directory events fall back to a full scan.
        if event.paths.is_empty() {
            self.need_full = true;
            return;
        }
        let managed_sync_event = event.paths.iter().all(|path| {
            path.components()
                .any(|component| component.as_os_str() == ".tine-sync")
        });
        if event_is_plain_page_file_op(&event) || managed_sync_event {
            self.paths.extend(event.paths);
        } else {
            self.need_full = true;
        }
    }
}

fn is_page_file_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|x| x.to_str()),
        Some("md") | Some("org")
    )
}

fn path_is_existing_dir(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

fn all_concrete_page_file_paths(event: &notify::Event) -> bool {
    !event.paths.is_empty()
        && event
            .paths
            .iter()
            .all(|p| is_page_file_path(p) && !path_is_existing_dir(p))
}

fn event_is_plain_page_file_op(event: &notify::Event) -> bool {
    use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};

    if !all_concrete_page_file_paths(event) {
        return false;
    }
    match event.kind {
        EventKind::Create(CreateKind::File) => true,
        EventKind::Modify(ModifyKind::Data(_)) | EventKind::Modify(ModifyKind::Metadata(_)) => true,
        EventKind::Modify(ModifyKind::Name(
            RenameMode::From | RenameMode::To | RenameMode::Both,
        )) => true,
        EventKind::Remove(RemoveKind::File) => true,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: SystemTime,
    len: u64,
    identity: u128,
    changed: i128,
}

fn metadata_stamp(md: &std::fs::Metadata) -> Option<FileStamp> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return Some(FileStamp {
            modified: md.modified().ok()?,
            len: md.len(),
            identity: ((md.dev() as u128) << 64) | md.ino() as u128,
            changed: (md.ctime() as i128) * 1_000_000_000 + md.ctime_nsec() as i128,
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return Some(FileStamp {
            modified: md.modified().ok()?,
            len: md.len(),
            identity: md.creation_time() as u128,
            changed: md.last_write_time() as i128,
        });
    }
    #[cfg(not(any(unix, windows)))]
    Some(FileStamp {
        modified: md.modified().ok()?,
        len: md.len(),
        identity: md
            .created()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_nanos(),
        changed: 0,
    })
}

/// Recursively collect every `.md`/`.org` page file under `dir` with its
/// (mtime, len) — the watcher's diff snapshot. Descends sub-directories so a page
/// in a sub-folder (#21) is reconciled like a top-level one; mirrors the core's
/// `list_md` walk: match page files by extension (the metadata read is needed for
/// mtime/len anyway), skip hidden dirs and symlinked dirs (no cycles, no escaping
/// the watched tree). Scoped to the dir passed in (journals/ or pages/).
fn collect_page_files(dir: &std::path::Path, out: &mut HashMap<PathBuf, FileStamp>) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(file_type) = e.file_type() else {
                continue;
            };
            if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("md") | Some("org")
            ) {
                // Never follow a page-looking symlink. Besides cycles, a
                // `secret.md` symlink could otherwise expose outside bytes.
                if file_type.is_file() {
                    let Ok(md) = e.metadata() else { continue };
                    if let Some(stamp) = metadata_stamp(&md) {
                        out.insert(p, stamp);
                    }
                }
                continue;
            }
            let hidden = p
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with('.'))
                .unwrap_or(true);
            if !hidden && file_type.is_dir() {
                stack.push(p);
            }
        }
    }
}

fn collect_graph_page_files(dirs: &[PathBuf; 2]) -> HashMap<PathBuf, FileStamp> {
    let mut current: HashMap<PathBuf, FileStamp> = HashMap::new();
    for dir in dirs {
        collect_page_files(dir, &mut current);
    }
    current
}

fn file_snapshot(path: &Path) -> Option<FileStamp> {
    let md = std::fs::metadata(path).ok()?;
    if !md.is_file() {
        return None;
    }
    metadata_stamp(&md)
}

fn full_diff_reconcile(
    graph: &Graph,
    snap: &mut HashMap<PathBuf, FileStamp>,
    mut current: HashMap<PathBuf, FileStamp>,
) -> (Vec<GraphChange>, bool, Vec<String>) {
    let mut changes: Vec<GraphChange> = Vec::new();
    let mut errors = Vec::new();
    let mut failed_paths = Vec::new();
    // A sync-tool conflict copy appearing/vanishing isn't a page change (it's
    // never cached), but the conflicts panel must refresh — track it and emit
    // `conflicts-changed` once.
    let mut conflicts_dirty = false;
    for (p, m) in &current {
        if snap.get(p) != Some(m) {
            if tine_core::model::path_is_sync_conflict(p) {
                conflicts_dirty = true;
            } else {
                match graph.sync_file_checked(p) {
                    Ok(Some(en)) => changes.push(GraphChange {
                        name: en.name,
                        kind: en.kind,
                        removed: false,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        errors.push(format!("{}: {error}", p.display()));
                        failed_paths.push(p.clone());
                    }
                }
            }
        }
    }
    for p in snap.keys() {
        if !current.contains_key(p) {
            if tine_core::model::path_is_sync_conflict(p) {
                conflicts_dirty = true;
            } else {
                match graph.sync_deleted_file(p) {
                    Ok(Some(en)) => changes.push(GraphChange {
                        name: en.name,
                        kind: en.kind,
                        removed: true,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        errors.push(format!("{}: {error}", p.display()));
                        failed_paths.push(p.clone());
                    }
                }
            }
        }
    }
    for path in failed_paths {
        match snap.get(&path).copied() {
            Some(previous) => {
                current.insert(path, previous);
            }
            None => {
                current.remove(&path);
            }
        }
    }
    *snap = current;
    (changes, conflicts_dirty, errors)
}

fn incremental_reconcile(
    graph: &Graph,
    snap: &mut HashMap<PathBuf, FileStamp>,
    paths: &HashSet<PathBuf>,
) -> (Vec<GraphChange>, bool, Vec<String>) {
    let mut changes: Vec<GraphChange> = Vec::new();
    let mut conflicts_dirty = false;
    let mut errors = Vec::new();

    // Reconcile present destinations before absent sources. A provider-delivered
    // external rename then lets the new path claim persisted block IDs before the
    // old page is tombstoned, preserving identity across the two snapshot events.
    let mut ordered: Vec<&PathBuf> = paths.iter().collect();
    ordered.sort_by_key(|path| file_snapshot(path).is_none());
    for p in ordered {
        if let Some(m) = file_snapshot(p) {
            // This path came from an explicit OS event. Always compare its
            // content even if a sync/copy tool preserved mtime and length;
            // the graph reconciliation already suppresses Tine's own/unchanged bytes.
            if tine_core::model::path_is_sync_conflict(p) {
                conflicts_dirty = true;
            } else {
                match graph.sync_file_checked(p) {
                    Ok(Some(en)) => changes.push(GraphChange {
                        name: en.name,
                        kind: en.kind,
                        removed: false,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        errors.push(format!("{}: {error}", p.display()));
                        continue;
                    }
                }
            }
            snap.insert(p.clone(), m);
        } else if snap.contains_key(p) {
            if tine_core::model::path_is_sync_conflict(p) {
                conflicts_dirty = true;
            } else {
                match graph.sync_deleted_file(p) {
                    Ok(Some(en)) => changes.push(GraphChange {
                        name: en.name,
                        kind: en.kind,
                        removed: true,
                    }),
                    Ok(None) => {}
                    Err(error) => {
                        errors.push(format!("{}: {error}", p.display()));
                        continue;
                    }
                }
            }
            snap.remove(p);
        }
    }

    (changes, conflicts_dirty, errors)
}

fn reconcile_pending(
    graph: &Graph,
    dirs: &[PathBuf; 2],
    snap: &mut HashMap<PathBuf, FileStamp>,
    paths: &HashSet<PathBuf>,
    need_full: bool,
) -> (Vec<GraphChange>, bool, bool, Vec<String>) {
    if need_full || paths.is_empty() {
        let current = collect_graph_page_files(dirs);
        let (changes, conflicts_dirty, errors) = full_diff_reconcile(graph, snap, current);
        (changes, conflicts_dirty, true, errors)
    } else {
        let (changes, conflicts_dirty, errors) = incremental_reconcile(graph, snap, paths);
        (changes, conflicts_dirty, false, errors)
    }
}

fn pending_for_graph(paths: &HashSet<PathBuf>, dirs: &[PathBuf; 2]) -> HashSet<PathBuf> {
    paths
        .iter()
        .filter(|path| dirs.iter().any(|dir| path.starts_with(dir)))
        .cloned()
        .collect()
}

/// Watch the graph dirs for external changes (Logseq, Syncthing) and reconcile
/// them into the cache, emitting `graph-changed` so the UI can reload. Two
/// mechanisms, switchable at runtime via the device-local `watch_mode` setting:
///
///   - **"inotify" (default):** a real OS filesystem watcher (the `notify`
///     crate — inotify on Linux). Idle = *zero* periodic wakeups; the thread
///     blocks until the kernel reports a change. Matches OG Logseq (chokidar)
///     and is the right choice on a normal local disk.
///   - **"poll":** a 3-second mtime scan. Robust on filesystems where inotify is
///     unreliable (some NFS / network mounts), at the cost of constant periodic
///     wakeups. Use this only when inotify misses external edits.
///
/// In both modes the reconcile is identical and suppresses Tine's *own* writes
/// via the cache comparison inside `sync_file`. A control channel (poked by
/// `load_graph` on a graph switch and by `set_watch_mode`) lets the thread
/// re-target or switch mechanism at once, without polling for those either.
pub(crate) fn start_watcher(app: tauri::AppHandle) {
    use notify::Watcher;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let pending = Arc::new(Mutex::new(Pending::default()));
    if let Ok(mut slot) = app.state::<AppState>().watch_ctl.lock() {
        *slot = Some(tx.clone());
    }
    std::thread::spawn(move || {
        struct WatchedGraph {
            slot: Arc<GraphSlot>,
            dirs: [PathBuf; 2],
            sync_dir: PathBuf,
            snap: HashMap<PathBuf, FileStamp>,
            baseline: bool,
            last_sync_error: Option<String>,
        }

        let mut graphs: HashMap<String, WatchedGraph> = HashMap::new();
        let mut watcher: Option<notify::RecommendedWatcher> = None;
        let mut watched: HashSet<PathBuf> = HashSet::new();
        loop {
            let inotify = watch_mode(&app) != "poll";
            let entries = app.state::<AppState>().graphs.read().unwrap().entries();
            let live: HashSet<String> = entries.iter().map(|(label, _)| label.clone()).collect();
            graphs.retain(|label, _| live.contains(label));
            for (label, slot) in entries {
                let dirs = [slot.graph.journals_path(), slot.graph.pages_path()];
                let sync_dir = slot.graph.managed_sync_store_path();
                match graphs.get_mut(&label) {
                    Some(current) if current.dirs == dirs && current.sync_dir == sync_dir => {
                        current.slot = slot
                    }
                    _ => {
                        graphs.insert(
                            label,
                            WatchedGraph {
                                slot,
                                dirs,
                                sync_dir,
                                snap: HashMap::new(),
                                baseline: false,
                                last_sync_error: None,
                            },
                        );
                    }
                }
            }

            let desired: HashSet<PathBuf> = graphs
                .values()
                .flat_map(|graph| {
                    graph
                        .dirs
                        .iter()
                        .cloned()
                        .chain(graph.sync_dir.is_dir().then(|| graph.sync_dir.clone()))
                })
                .collect();

            // Bring the OS watcher in line with the current mode + dirs.
            if inotify {
                if watcher.is_none() {
                    let txc = tx.clone();
                    let pendingc = pending.clone();
                    watcher =
                        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                            if let Ok(mut p) = pendingc.lock() {
                                match res {
                                    Ok(event) => p.add_event(event),
                                    Err(_) => p.need_full = true,
                                }
                            }
                            let _ = txc.send(());
                        })
                        .ok();
                    watched.clear();
                }
                if let Some(w) = watcher.as_mut() {
                    for dir in watched.difference(&desired).cloned().collect::<Vec<_>>() {
                        let _ = w.unwatch(&dir);
                        watched.remove(&dir);
                    }
                    for dir in desired.difference(&watched).cloned().collect::<Vec<_>>() {
                        // Recursive so nested graph pages wake the reconcile.
                        if w.watch(&dir, notify::RecursiveMode::Recursive).is_ok() {
                            watched.insert(dir);
                        }
                    }
                }
            } else if watcher.is_some() {
                watcher = None; // poll mode → release the OS watcher
                watched.clear();
            }

            // --- reconcile (identical in both modes) ---
            let (paths, event_need_full) = if inotify {
                if let Ok(mut p) = pending.lock() {
                    let paths = std::mem::take(&mut p.paths);
                    let need_full = p.need_full;
                    p.need_full = false;
                    (paths, need_full)
                } else {
                    (HashSet::new(), true)
                }
            } else {
                (HashSet::new(), true)
            };
            for (label, graph) in graphs.iter_mut() {
                if !graph.baseline {
                    graph.snap = collect_graph_page_files(&graph.dirs);
                    graph.baseline = true;
                    continue;
                }
                let owned = pending_for_graph(&paths, &graph.dirs);
                let need_full = event_need_full || !inotify;
                let sync_dirty =
                    need_full || paths.iter().any(|path| path.starts_with(&graph.sync_dir));
                let mut sync_conflicts_dirty = false;
                if sync_dirty && graph.sync_dir.is_dir() {
                    match graph.slot.graph.pull_managed_sync() {
                        Ok(pull) => {
                            graph.last_sync_error = None;
                            for change in pull.changes {
                                let _ = app.emit_to(
                                    label,
                                    "graph-changed",
                                    GraphChange {
                                        name: change.entry.name,
                                        kind: change.entry.kind,
                                        removed: change.removed,
                                    },
                                );
                            }
                            sync_conflicts_dirty = pull.conflicts_changed;
                        }
                        Err(error) => {
                            let message = error.to_string();
                            if graph.last_sync_error.as_deref() != Some(&message) {
                                let _ = app.emit_to(label, "managed-sync-error", &message);
                                graph.last_sync_error = Some(message);
                            }
                        }
                    }
                }
                if need_full || !owned.is_empty() {
                    let (changes, conflicts_dirty, _, errors) = reconcile_pending(
                        &graph.slot.graph,
                        &graph.dirs,
                        &mut graph.snap,
                        &owned,
                        need_full,
                    );
                    for change in changes {
                        let _ = app.emit_to(label, "graph-changed", change);
                    }
                    if !errors.is_empty() {
                        let message = errors.join("; ");
                        if graph.last_sync_error.as_deref() != Some(&message) {
                            let _ = app.emit_to(label, "managed-sync-error", &message);
                            graph.last_sync_error = Some(message);
                        }
                    }
                    if conflicts_dirty || sync_conflicts_dirty {
                        let _ = app.emit_to(label, "conflicts-changed", ());
                    }
                } else if sync_conflicts_dirty {
                    let _ = app.emit_to(label, "conflicts-changed", ());
                }
            }

            // --- wait for the next cycle ---
            if inotify && !watched.is_empty() {
                // Block until the kernel reports a change (or a control poke).
                // Coalesce the several events produced by one atomic save.
                if rx.recv().is_ok() {
                    std::thread::sleep(Duration::from_millis(200));
                    while rx.try_recv().is_ok() {}
                }
            } else {
                std::thread::sleep(Duration::from_secs(3));
                while rx.try_recv().is_ok() {}
            }
        }
    });
}

/// How the file-watcher detects external changes (device-local, in
/// tine-settings.json): "inotify" (default on desktop) → a real OS watcher, no
/// idle wakeups; "poll" (default on Android) → a 3s mtime scan for filesystems
/// where inotify is flaky (some NFS, Android shared storage). See `start_watcher`.
fn watch_mode(app: &tauri::AppHandle) -> String {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("watch_mode")
                .and_then(|x| x.as_str().map(String::from))
        })
        .filter(|m| m == "poll" || m == "inotify")
        .unwrap_or_else(|| {
            if cfg!(target_os = "android") {
                "poll".to_string()
            } else {
                "inotify".to_string()
            }
        })
}

#[tauri::command]
pub(crate) fn get_watch_mode(app: tauri::AppHandle) -> String {
    watch_mode(&app)
}

#[tauri::command]
pub(crate) fn set_watch_mode(
    mode: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mode = if mode == "poll" { "poll" } else { "inotify" };
    update_settings(&app, |json| {
        json["watch_mode"] = serde_json::json!(mode);
    })?;
    // Wake the watcher so it switches mechanism right away.
    if let Some(tx) = state.watch_ctl.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn pending_paths_are_dispatched_only_to_the_owning_graph() {
        let paths = HashSet::from([
            PathBuf::from("/graphs/a/pages/one.md"),
            PathBuf::from("/graphs/b/journals/2026_07_10.md"),
        ]);
        let a = [
            PathBuf::from("/graphs/a/journals"),
            PathBuf::from("/graphs/a/pages"),
        ];
        let b = [
            PathBuf::from("/graphs/b/journals"),
            PathBuf::from("/graphs/b/pages"),
        ];
        assert_eq!(pending_for_graph(&paths, &a).len(), 1);
        assert_eq!(pending_for_graph(&paths, &b).len(), 1);
        assert!(pending_for_graph(&paths, &a)
            .iter()
            .all(|path| path.starts_with("/graphs/a")));
    }

    struct TempGraph {
        root: PathBuf,
    }

    impl TempGraph {
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "tine-watch-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("journals")).unwrap();
            std::fs::create_dir_all(root.join("pages")).unwrap();
            Self { root }
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }

        fn write(&self, rel: &str, content: &str) {
            let path = self.path(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }

        fn remove(&self, rel: &str) {
            std::fs::remove_file(self.path(rel)).unwrap();
        }

        fn rename(&self, from: &str, to: &str) {
            let to = self.path(to);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::rename(self.path(from), to).unwrap();
        }
    }

    impl Drop for TempGraph {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn graph_dirs(graph: &Graph) -> [PathBuf; 2] {
        [graph.journals_path(), graph.pages_path()]
    }

    fn warm_cache(graph: &Graph) {
        let _ = graph.search("__watcher_warm_cache__", 1);
    }

    fn sorted_changes(mut changes: Vec<GraphChange>) -> Vec<GraphChange> {
        fn kind_key(kind: PageKind) -> &'static str {
            match kind {
                PageKind::Journal => "journal",
                PageKind::Page => "page",
            }
        }
        changes.sort_by(|a, b| {
            (a.removed, kind_key(a.kind), a.name.as_str()).cmp(&(
                b.removed,
                kind_key(b.kind),
                b.name.as_str(),
            ))
        });
        changes
    }

    fn rel_paths(tg: &TempGraph, rels: &[&str]) -> HashSet<PathBuf> {
        rels.iter().map(|rel| tg.path(rel)).collect()
    }

    fn assert_incremental_matches_full(
        name: &str,
        setup: impl FnOnce(&TempGraph),
        mutate: impl FnOnce(&TempGraph) -> HashSet<PathBuf>,
    ) {
        let tg = TempGraph::new(name);
        setup(&tg);

        let inc_graph = Graph::open(&tg.root);
        let full_graph = Graph::open(&tg.root);
        warm_cache(&inc_graph);
        warm_cache(&full_graph);

        let dirs = graph_dirs(&inc_graph);
        let mut inc_snap = collect_graph_page_files(&dirs);
        let mut full_snap = inc_snap.clone();

        let paths = mutate(&tg);

        let (inc_changes, inc_conflicts_dirty, inc_errors) =
            incremental_reconcile(&inc_graph, &mut inc_snap, &paths);
        let fresh = collect_graph_page_files(&dirs);
        let (full_changes, full_conflicts_dirty, full_errors) =
            full_diff_reconcile(&full_graph, &mut full_snap, fresh.clone());

        assert_eq!(inc_snap, fresh, "incremental snap must match full scan");
        assert_eq!(full_snap, fresh, "full snap must match fresh scan");
        assert_eq!(inc_conflicts_dirty, full_conflicts_dirty);
        assert!(inc_errors.is_empty());
        assert!(full_errors.is_empty());
        assert_eq!(
            sorted_changes(inc_changes),
            sorted_changes(full_changes),
            "incremental changes must match full-diff changes"
        );
    }

    #[test]
    fn collect_page_files_descends_subdirectories() {
        // #21: the watcher snapshot must include page files in sub-folders, so an
        // edit/create there is reconciled (not invisible until a graph reopen).
        let dir = std::env::temp_dir().join(format!("tine-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Archive/Deep/Deeper")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("top.md"), "- t\n").unwrap();
        std::fs::write(dir.join("Archive/mid.org"), "* m\n").unwrap();
        std::fs::write(dir.join("Archive/Deep/Deeper/deep.md"), "- d\n").unwrap();
        std::fs::write(dir.join("Archive/notes.txt"), "ignored\n").unwrap();
        std::fs::write(dir.join(".hidden/skip.md"), "- s\n").unwrap();

        let mut out: HashMap<PathBuf, FileStamp> = HashMap::new();
        collect_page_files(&dir, &mut out);
        let mut names: Vec<String> = out
            .keys()
            .map(|p| {
                p.strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["Archive/Deep/Deeper/deep.md", "Archive/mid.org", "top.md"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn collect_page_files_does_not_follow_page_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!("tine-watch-link-{}", std::process::id()));
        let outside =
            std::env::temp_dir().join(format!("tine-watch-outside-{}.md", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&outside, "- outside\n").unwrap();
        symlink(&outside, dir.join("secret.md")).unwrap();

        let mut out = HashMap::new();
        collect_page_files(&dir, &mut out);
        assert!(out.is_empty());

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_file(&outside).ok();
    }

    #[test]
    fn incremental_create_top_level_file_matches_full_diff() {
        assert_incremental_matches_full(
            "create-top",
            |tg| tg.write("pages/Seed.md", "- seed\n"),
            |tg| {
                tg.write("pages/New.md", "- new\n");
                rel_paths(tg, &["pages/New.md"])
            },
        );
    }

    #[test]
    fn incremental_create_nested_file_matches_full_diff() {
        assert_incremental_matches_full(
            "create-nested",
            |tg| tg.write("pages/Seed.md", "- seed\n"),
            |tg| {
                tg.write("pages/sub/New.md", "- nested\n");
                rel_paths(tg, &["pages/sub/New.md"])
            },
        );
    }

    #[test]
    fn incremental_modify_len_change_matches_full_diff() {
        assert_incremental_matches_full(
            "modify-len",
            |tg| tg.write("pages/Edit.md", "- one\n"),
            |tg| {
                std::thread::sleep(Duration::from_millis(20));
                tg.write("pages/Edit.md", "- one\n- two\n");
                rel_paths(tg, &["pages/Edit.md"])
            },
        );
    }

    #[test]
    fn incremental_modify_same_len_mtime_change_matches_full_diff() {
        assert_incremental_matches_full(
            "modify-same-len",
            |tg| tg.write("pages/Edit.md", "- alpha\n"),
            |tg| {
                std::thread::sleep(Duration::from_millis(20));
                tg.write("pages/Edit.md", "- beta!\n");
                rel_paths(tg, &["pages/Edit.md"])
            },
        );
    }

    #[test]
    fn explicit_event_reconciles_even_when_snapshot_metadata_is_equal() {
        let tg = TempGraph::new("explicit-same-metadata");
        tg.write("pages/Edit.md", "- alpha\n");
        let graph = Graph::open(&tg.root);
        warm_cache(&graph);
        let path = tg.path("pages/Edit.md");
        tg.write("pages/Edit.md", "- bravo\n"); // equal byte length
        let stamp = file_snapshot(&path).unwrap();
        // Simulate a sync tool preserving every snapshot field: the explicit
        // notify path must still reach Graph::sync_file's content comparison.
        let mut snap = HashMap::from([(path.clone(), stamp)]);
        let (changes, conflicts_dirty, errors) =
            incremental_reconcile(&graph, &mut snap, &HashSet::from([path]));
        assert!(!conflicts_dirty);
        assert!(errors.is_empty());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "Edit");
        assert_eq!(changes[0].kind, PageKind::Page);
        assert!(!changes[0].removed);
    }

    #[test]
    fn incremental_remove_top_level_file_matches_full_diff() {
        assert_incremental_matches_full(
            "remove-top",
            |tg| {
                tg.write("pages/Keep.md", "- keep\n");
                tg.write("pages/Delete.md", "- delete\n");
            },
            |tg| {
                tg.remove("pages/Delete.md");
                rel_paths(tg, &["pages/Delete.md"])
            },
        );
    }

    #[test]
    fn incremental_remove_nested_file_matches_full_diff() {
        assert_incremental_matches_full(
            "remove-nested",
            |tg| {
                tg.write("pages/Keep.md", "- keep\n");
                tg.write("pages/sub/Delete.md", "- delete\n");
            },
            |tg| {
                tg.remove("pages/sub/Delete.md");
                rel_paths(tg, &["pages/sub/Delete.md"])
            },
        );
    }

    #[test]
    fn incremental_rename_within_pages_matches_full_diff() {
        assert_incremental_matches_full(
            "rename-within-pages",
            |tg| tg.write("pages/Old.md", "- renamed\n"),
            |tg| {
                tg.rename("pages/Old.md", "pages/New.md");
                rel_paths(tg, &["pages/Old.md", "pages/New.md"])
            },
        );
    }

    #[test]
    fn incremental_rename_across_tree_matches_full_diff() {
        assert_incremental_matches_full(
            "rename-across-tree",
            |tg| tg.write("pages/JournalMove.md", "- moved\n"),
            |tg| {
                tg.rename("pages/JournalMove.md", "journals/2026_07_10.md");
                rel_paths(tg, &["pages/JournalMove.md", "journals/2026_07_10.md"])
            },
        );
    }

    #[test]
    fn incremental_burst_union_matches_full_diff() {
        assert_incremental_matches_full(
            "burst-union",
            |tg| {
                tg.write("pages/Edit.md", "- edit before\n");
                tg.write("pages/Delete.md", "- delete\n");
                tg.write("pages/Keep.md", "- keep\n");
            },
            |tg| {
                std::thread::sleep(Duration::from_millis(20));
                tg.write("pages/Edit.md", "- edit after\n");
                tg.remove("pages/Delete.md");
                tg.write("pages/Create.md", "- create\n");
                tg.write("pages/sub/Nested.md", "- nested\n");
                rel_paths(
                    tg,
                    &[
                        "pages/Edit.md",
                        "pages/Delete.md",
                        "pages/Create.md",
                        "pages/sub/Nested.md",
                    ],
                )
            },
        );
    }

    #[test]
    fn reconcile_pending_need_full_uses_full_scan_branch() {
        let tg = TempGraph::new("need-full");
        tg.write("pages/Seed.md", "- seed\n");

        let inc_graph = Graph::open(&tg.root);
        let full_graph = Graph::open(&tg.root);
        warm_cache(&inc_graph);
        warm_cache(&full_graph);

        let dirs = graph_dirs(&inc_graph);
        let mut inc_snap = collect_graph_page_files(&dirs);
        let mut full_snap = inc_snap.clone();

        tg.write(
            "pages/sub/CreatedByDirEvent.md",
            "- created through dir op\n",
        );
        let incomplete_paths = rel_paths(&tg, &["pages/Seed.md"]);
        let (inc_changes, inc_conflicts_dirty, used_full, inc_errors) =
            reconcile_pending(&inc_graph, &dirs, &mut inc_snap, &incomplete_paths, true);
        let fresh = collect_graph_page_files(&dirs);
        let (full_changes, full_conflicts_dirty, full_errors) =
            full_diff_reconcile(&full_graph, &mut full_snap, fresh.clone());

        assert!(used_full, "need_full must bypass incremental reconcile");
        assert!(inc_errors.is_empty());
        assert!(full_errors.is_empty());
        assert_eq!(inc_snap, fresh);
        assert_eq!(full_snap, fresh);
        assert_eq!(inc_conflicts_dirty, full_conflicts_dirty);
        assert_eq!(sorted_changes(inc_changes), sorted_changes(full_changes));
    }
}
