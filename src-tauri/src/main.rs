// Prevent a console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tine_core::model::{Graph, GraphMeta, PageDto, PageEntry, PageKind, RefGroup};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};
use tauri::{Emitter, Manager, State};

// The graph lives behind an RwLock holding an Arc, so read commands clone the
// Arc and release the lock immediately — a long read (search / query / asset
// read) no longer serializes every other command behind it. Only replacing the
// graph (open / switch) takes the write lock.
struct AppState {
    graph: RwLock<Option<Arc<Graph>>>,
    // Poke channel to the file-watcher thread: `load_graph` (graph switch) and
    // `set_watch_mode` send `()` so the watcher re-targets / switches mechanism
    // immediately instead of waiting for its next cycle. Set once by
    // `start_watcher`.
    watch_ctl: Mutex<Option<Sender<()>>>,
}

#[derive(Clone, serde::Serialize)]
struct GraphChange {
    name: String,
    kind: PageKind,
    removed: bool,
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
fn start_watcher(app: tauri::AppHandle) {
    use notify::Watcher;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    if let Ok(mut slot) = app.state::<AppState>().watch_ctl.lock() {
        *slot = Some(tx.clone());
    }
    std::thread::spawn(move || {
        let mut snap: HashMap<PathBuf, SystemTime> = HashMap::new();
        let mut baseline = false;
        let mut watcher: Option<notify::RecommendedWatcher> = None;
        let mut watched: Vec<PathBuf> = Vec::new();
        let mut last_dirs: Option<[PathBuf; 2]> = None;
        loop {
            let inotify = watch_mode(&app) != "poll";
            // Clone the graph Arc and release the lock immediately, so the
            // (potentially slow) reconcile below — directory scan + per-file
            // sync_file parses — never holds the lock that a graph switch needs.
            let (dirs, graph) = {
                let state: State<'_, AppState> = app.state();
                let g = state.graph.read().unwrap();
                match g.as_ref() {
                    Some(g) => (Some([g.journals_path(), g.pages_path()]), Some(g.clone())),
                    None => (None, None),
                }
            };

            // First load or graph switch: reset the diff baseline (so the new
            // graph's files aren't all reported as "added") and drop the stale
            // watches pointing at the previous graph.
            if dirs != last_dirs {
                snap.clear();
                baseline = false;
                last_dirs = dirs.clone();
                if let Some(w) = watcher.as_mut() {
                    for d in &watched {
                        let _ = w.unwatch(d);
                    }
                }
                watched.clear();
            }

            // Bring the OS watcher in line with the current mode + dirs.
            if inotify {
                if watcher.is_none() {
                    let txc = tx.clone();
                    watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                        // Any successful event wakes the loop for a full rescan;
                        // we don't trust per-event detail (renames arrive as
                        // remove+create, etc.) — the rescan is the source of truth.
                        if res.is_ok() {
                            let _ = txc.send(());
                        }
                    })
                    .ok();
                    watched.clear();
                }
                if let (Some(w), Some(ds)) = (watcher.as_mut(), dirs.as_ref()) {
                    if watched.is_empty() {
                        for d in ds.iter() {
                            if w.watch(d, notify::RecursiveMode::NonRecursive).is_ok() {
                                watched.push(d.clone());
                            }
                        }
                    }
                }
            } else if watcher.is_some() {
                watcher = None; // poll mode → release the OS watcher
                watched.clear();
            }

            // --- reconcile (identical in both modes) ---
            if let (Some(ds), Some(graph)) = (dirs.as_ref(), graph.as_ref()) {
                let mut current: HashMap<PathBuf, SystemTime> = HashMap::new();
                for dir in ds {
                    if let Ok(rd) = std::fs::read_dir(dir) {
                        for e in rd.flatten() {
                            let p = e.path();
                            // Watch markdown AND org page files (mirror the core's
                            // is_page_file), so external .org edits/creates/deletes
                            // are reconciled like .md ones.
                            if !matches!(p.extension().and_then(|x| x.to_str()), Some("md") | Some("org")) {
                                continue;
                            }
                            if let Ok(m) = e.metadata().and_then(|md| md.modified()) {
                                current.insert(p, m);
                            }
                        }
                    }
                }
                if !baseline {
                    snap = current;
                    baseline = true; // first scan establishes the baseline; emit nothing
                } else {
                    let mut changes: Vec<GraphChange> = Vec::new();
                    for (p, m) in &current {
                        if snap.get(p) != Some(m) {
                            if let Some(en) = graph.sync_file(p) {
                                changes.push(GraphChange { name: en.name, kind: en.kind, removed: false });
                            }
                        }
                    }
                    for p in snap.keys() {
                        if !current.contains_key(p) {
                            if let Some(en) = graph.forget_file(p) {
                                changes.push(GraphChange { name: en.name, kind: en.kind, removed: true });
                            }
                        }
                    }
                    snap = current;
                    for c in changes {
                        let _ = app.emit("graph-changed", c);
                    }
                }
            }

            // --- wait for the next cycle ---
            if inotify && !watched.is_empty() {
                // Block until the kernel reports a change (or a control poke).
                // Idle = no wakeups. Coalesce a burst (one save fires several
                // inotify events) into a single reconcile via a short settle.
                if rx.recv().is_ok() {
                    std::thread::sleep(Duration::from_millis(200));
                    while rx.try_recv().is_ok() {}
                }
            } else {
                // Poll mode, or inotify with nothing watched yet (no graph open):
                // a short sleep, draining any stray pokes so they don't pile up.
                std::thread::sleep(Duration::from_secs(3));
                while rx.try_recv().is_ok() {}
            }
        }
    });
}

/// Resolve the graph root: explicit path, else env var, else first CLI arg.
fn resolve_root(path: &str) -> Option<String> {
    if !path.is_empty() {
        return Some(path.to_string());
    }
    for var in ["TINE_GRAPH"] {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    std::env::args().nth(1)
}

#[tauri::command]
fn load_graph(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<GraphMeta, String> {
    let root = resolve_root(&path).ok_or_else(|| {
        "no graph path provided (set TINE_GRAPH or pass a path)".to_string()
    })?;
    let graph = Graph::open(&root);
    let meta = graph.meta();
    // Recover any journals mis-saved under their title (see method docs).
    graph.migrate_journal_filenames();
    *state.graph.write().unwrap() = Some(Arc::new(graph));
    // Nudge the watcher so it re-targets the new graph's dirs at once (in inotify
    // mode it's otherwise blocked on the old graph's events).
    if let Some(tx) = state.watch_ctl.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
    backup_async(app.clone());
    warm_cache_async(app);
    Ok(meta)
}

// Snapshot the graph's markdown into the OS app-data dir on open, keeping the
// last few. Local-only (outside the graph, so Syncthing never sees it); a safety
// net against a bad write or accidental edit. Best-effort and fully detached so
// it never blocks startup or holds the graph lock during file copies.
const BACKUP_KEEP_DEFAULT: usize = 12;

fn backup_async(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // Defer the launch snapshot ~1s so its whole-graph file copy doesn't
        // contend for disk I/O with first-journal paint and the warm-cache parse
        // at open (felt on slow/NFS disks or a throttled laptop). Safe: the
        // snapshot guards this session's edits, and the user hasn't edited yet in
        // the first second — the on-disk files are still intact — so a crash in
        // that window loses nothing the snapshot would have protected.
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let _ = do_backup(&app, ""); // launch snapshot is best-effort
    });
}

/// Take one snapshot of the current graph now (synchronous). Returns the number
/// of files copied (0 = nothing to back up). Reads the keep count from the local
/// app-settings file and prunes old snapshots afterwards. `suffix` tags special
/// snapshots (e.g. "pre-restore") so they get a distinct, collision-proof
/// directory name and are exempt from the keep-count prune.
/// Returns (files copied, complete) — `complete` is false if ANY `.md`/config
/// copy failed, so the caller (restore) can refuse to proceed without a full
/// rollback snapshot.
fn do_backup(app: &tauri::AppHandle, suffix: &str) -> (usize, bool) {
    // Grab just the paths under the lock, then copy from disk lock-free.
    let (journals, pages, cfg, root) = {
        let state: State<'_, AppState> = app.state();
        let guard = state.graph.read().unwrap();
        match guard.as_ref() {
            Some(g) => (
                g.journals_path(),
                g.pages_path(),
                g.root.join("logseq").join("config.edn"),
                g.root.clone(),
            ),
            None => return (0, false),
        }
    };
    let Ok(data_dir) = app.path().app_data_dir() else { return (0, false) };
    let base = data_dir
        .join("backups")
        .join(sanitize_id(&root.display().to_string()));
    let stamp = backup_stamp();
    let name = if suffix.is_empty() { stamp } else { format!("{stamp}-{suffix}") };
    // Reserve a UNIQUE destination directory. The stamp is second-granularity, so
    // two snapshots in the same second (e.g. a launch snapshot racing a pre-restore
    // snapshot) would otherwise share one directory — and copy_md_dir, which copies
    // in but never removes files absent from the live graph, would mix both
    // snapshots' files, leaving a later restore with stale notes. `create_dir`
    // (non-recursive) fails atomically if the name is taken, so we bump a counter
    // until we win an unused name.
    let _ = std::fs::create_dir_all(&base);
    let mut dest = base.join(&name);
    let mut k = 2;
    loop {
        match std::fs::create_dir(&dest) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                dest = base.join(format!("{name}-{k}"));
                k += 1;
            }
            Err(_) => break, // other error → fall through; copy below is best-effort
        }
    }
    let (cj, fj) = copy_md_dir(&journals, &dest.join(dir_name(&journals)));
    let (cp, fp) = copy_md_dir(&pages, &dest.join(dir_name(&pages)));
    let mut n = cj + cp;
    let mut failed = fj + fp;
    if cfg.exists() {
        let out = dest.join("logseq");
        if std::fs::create_dir_all(&out).is_ok()
            && std::fs::copy(&cfg, out.join("config.edn")).is_ok()
        {
            n += 1;
        } else {
            failed += 1;
        }
    }
    if n == 0 {
        let _ = std::fs::remove_dir_all(&dest);
        return (0, failed == 0);
    }
    prune_backups(&base, backup_keep(app));
    (n, failed == 0)
}

// --- local app settings (outside the graph): currently just the backup keep
// count. A tiny JSON file in the OS app-data dir. ---
fn settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("tine-settings.json"))
}
fn backup_keep(app: &tauri::AppHandle) -> usize {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("backup_keep").and_then(|x| x.as_u64()))
        .map(|n| (n as usize).max(1))
        .unwrap_or(BACKUP_KEEP_DEFAULT)
}

#[derive(serde::Serialize)]
struct BackupInfo {
    stamp: String,
    files: usize,
}

#[tauri::command]
fn get_backup_keep(app: tauri::AppHandle) -> usize {
    backup_keep(&app)
}

#[tauri::command]
fn set_backup_keep(keep: usize, app: tauri::AppHandle) -> Result<(), String> {
    let keep = keep.clamp(1, 1000);
    let p = settings_path(&app).ok_or("no app-data dir")?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Merge into the existing settings (don't clobber other keys, e.g.
    // capture_enter_files).
    let mut json = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    json["backup_keep"] = serde_json::json!(keep);
    std::fs::write(&p, serde_json::to_string_pretty(&json).unwrap())
        .map_err(|e| e.to_string())?;
    // Apply the new (possibly lower) cap to the current graph's snapshots now.
    if let Some(base) = backup_base(&app) {
        prune_backups(&base, keep);
    }
    Ok(())
}

/// Quick-capture Enter behaviour (app-level, in tine-settings.json): true → a
/// plain Enter files the capture; false (default) → Enter makes a new block and
/// Cmd/Ctrl+Enter files.
fn capture_enter_files(app: &tauri::AppHandle) -> bool {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("capture_enter_files").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

#[tauri::command]
fn get_capture_enter_files(app: tauri::AppHandle) -> bool {
    capture_enter_files(&app)
}

#[tauri::command]
fn set_capture_enter_files(value: bool, app: tauri::AppHandle) -> Result<(), String> {
    let p = settings_path(&app).ok_or("no app-data dir")?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut json = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    json["capture_enter_files"] = serde_json::Value::Bool(value);
    std::fs::write(&p, serde_json::to_string_pretty(&json).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

/// How the file-watcher detects external changes (device-local, in
/// tine-settings.json): "inotify" (default) → a real OS watcher, no idle
/// wakeups; "poll" → a 3s mtime scan for filesystems where inotify is flaky
/// (some NFS). See `start_watcher`.
fn watch_mode(app: &tauri::AppHandle) -> String {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("watch_mode").and_then(|x| x.as_str().map(String::from)))
        .filter(|m| m == "poll" || m == "inotify")
        .unwrap_or_else(|| "inotify".to_string())
}

#[tauri::command]
fn get_watch_mode(app: tauri::AppHandle) -> String {
    watch_mode(&app)
}

#[tauri::command]
fn set_watch_mode(mode: String, app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mode = if mode == "poll" { "poll" } else { "inotify" };
    let p = settings_path(&app).ok_or("no app-data dir")?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut json = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    json["watch_mode"] = serde_json::json!(mode);
    std::fs::write(&p, serde_json::to_string_pretty(&json).unwrap()).map_err(|e| e.to_string())?;
    // Wake the watcher so it switches mechanism right away.
    if let Some(tx) = state.watch_ctl.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
    Ok(())
}

/// Path to the persisted UI session (open tabs / active tab / zoom). This is
/// app-level window state, not graph content, so it lives next to the settings
/// file in the app-data dir. (WebKitGTK's localStorage is not durably persisted
/// for this app, so the frontend can't rely on it — it round-trips here.)
fn session_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("tine-session.json"))
}

#[tauri::command]
fn load_session(app: tauri::AppHandle) -> Option<String> {
    std::fs::read_to_string(session_path(&app)?).ok()
}

#[tauri::command]
fn save_session(data: String, app: tauri::AppHandle) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Unique temp name per write so two concurrent saves (a burst of tab actions)
    // can't clobber each other's temp file before the rename.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let p = session_path(&app).ok_or("no app-data dir")?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = p.with_extension(format!("json.tmp{seq}"));
    // Write to a temp file then atomically rename, so a crash mid-write can never
    // leave a truncated session that fails to parse.
    std::fs::write(&tmp, data.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    Ok(())
}

/// The backup directory for the currently-open graph (`<app-data>/backups/<id>`).
fn backup_base(app: &tauri::AppHandle) -> Option<PathBuf> {
    let root = {
        let state: State<'_, AppState> = app.state();
        let guard = state.graph.read().unwrap();
        guard.as_ref().map(|g| g.root.clone())?
    };
    let data_dir = app.path().app_data_dir().ok()?;
    Some(data_dir.join("backups").join(sanitize_id(&root.display().to_string())))
}

#[tauri::command]
fn list_backups(app: tauri::AppHandle) -> Result<Vec<BackupInfo>, String> {
    let Some(base) = backup_base(&app) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let stamp = match p.file_name().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let files = count_md_recursive(&p);
            out.push(BackupInfo { stamp, files });
        }
    }
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp)); // newest first
    Ok(out)
}

fn count_md_recursive(dir: &std::path::Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                n += count_md_recursive(&p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                n += 1;
            }
        }
    }
    n
}

/// Restore a snapshot into the live graph, overwriting `journals/`, `pages/` and
/// `config.edn`. Takes a fresh safety snapshot of the *current* state first (so a
/// mistaken restore is itself reversible). Destructive — the frontend confirms.
#[tauri::command]
fn restore_backup(stamp: String, app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // Guard against path traversal — a stamp is only ever `YYYY-MM-DD_HH-MM-SS`.
    if stamp.is_empty()
        || !stamp.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("invalid backup id".into());
    }
    let (journals, pages, cfg_dest) = {
        let guard = state.graph.read().unwrap();
        let g = guard.as_ref().ok_or("no graph loaded")?;
        (
            g.journals_path(),
            g.pages_path(),
            g.root.join("logseq").join("config.edn"),
        )
    };
    let base = backup_base(&app).ok_or("no app-data dir")?;
    let src = base.join(&stamp);
    if !src.is_dir() {
        return Err("backup not found".into());
    }
    // Safety net: snapshot the current (pre-restore) state first, under a distinct
    // name so it can't collide with (or be pruned by) the launch snapshot the
    // post-restore reload will take. Abort if the snapshot fails while the live
    // graph has content — never run a destructive restore without a way back.
    let (snapshot_n, complete) = do_backup(&app, "pre-restore");
    let live_n = count_md_recursive(&journals) + count_md_recursive(&pages);
    // A destructive restore must be fully reversible: abort unless the pre-restore
    // snapshot captured everything (every file copied, nothing skipped).
    if live_n > 0 && (snapshot_n == 0 || !complete) {
        return Err("couldn't create a complete pre-restore safety snapshot — restore aborted".into());
    }
    // Restore each dir; a copy failure aborts WITHOUT having deleted anything
    // (copy-in happens before delete-extras), so a failure never loses data.
    restore_md_dir(&src.join(dir_name(&journals)), &journals)
        .map_err(|e| format!("restore journals failed: {e}"))?;
    restore_md_dir(&src.join(dir_name(&pages)), &pages)
        .map_err(|e| format!("restore pages failed: {e}"))?;
    let src_cfg = src.join("logseq").join("config.edn");
    if src_cfg.exists() {
        if let Some(parent) = cfg_dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::copy(&src_cfg, &cfg_dest).map_err(|e| format!("restore config failed: {e}"))?;
    }
    Ok(())
}

/// Restore the `.md` files in `dest` from `src`. Each file is copied to a temp
/// file in `dest` then atomically renamed over the target — so a failure or
/// power-loss mid-copy can never leave a live note truncated/half-written. Copies
/// happen FIRST; only after they all succeed do we delete `dest` `.md` files not
/// in the backup. A copy error returns early leaving a superset of files (no
/// data lost). Non-`.md` files are left untouched.
fn restore_md_dir(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dest)?;
    let mut restored: std::collections::HashSet<String> = std::collections::HashSet::new();
    for e in std::fs::read_dir(src)?.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("md") {
            if let Some(name) = p.file_name() {
                let target = dest.join(name);
                let tmp = dest.join(format!(".{}.tine-restore", name.to_string_lossy()));
                std::fs::copy(&p, &tmp)?;
                std::fs::rename(&tmp, &target)?; // atomic replace on the same fs
                restored.insert(name.to_string_lossy().into_owned());
            }
        }
    }
    for e in std::fs::read_dir(dest)?.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("md") {
            if let Some(name) = p.file_name() {
                if !restored.contains(name.to_string_lossy().as_ref()) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
    Ok(())
}

fn dir_name(p: &std::path::Path) -> String {
    p.file_name().and_then(|s| s.to_str()).unwrap_or("dir").to_string()
}
fn sanitize_id(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect()
}
/// Copy every `.md` from `src` to `dest`. Returns (copied, failed) so the caller
/// can tell a complete snapshot from a partial one.
fn copy_md_dir(src: &std::path::Path, dest: &std::path::Path) -> (usize, usize) {
    // Materialize the dest dir up front, even when src has no .md files — so the
    // snapshot records "this dir existed and was empty". Otherwise restore can't
    // tell an empty-at-backup dir from a missing one, and leaves destination .md
    // extras in place (mixing current files into the restored snapshot).
    let _ = std::fs::create_dir_all(dest);
    let rd = match std::fs::read_dir(src) {
        Ok(rd) => rd,
        // A genuinely-absent source dir (e.g. a graph with no pages/) is not a
        // failure — there's nothing to snapshot. But a dir we CAN'T read
        // (permission / I/O) MUST count as failed, so a pre-restore safety
        // snapshot isn't falsely reported complete and a destructive restore can
        // refuse to proceed without a trustworthy rollback.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (0, 0),
        Err(_) => return (0, 1),
    };
    let (mut copied, mut failed) = (0usize, 0usize);
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let Some(name) = p.file_name() else { continue };
        if std::fs::create_dir_all(dest).is_ok() && std::fs::copy(&p, dest.join(name)).is_ok() {
            copied += 1;
        } else {
            failed += 1;
        }
    }
    (copied, failed)
}
fn prune_backups(base: &std::path::Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(base) else { return };
    // Only the routine launch snapshots are subject to the keep-count. Tagged
    // snapshots (e.g. "...-pre-restore") are deliberate safety points and are
    // never auto-pruned.
    let mut dirs: Vec<std::path::PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.contains("-pre-restore"))
                    .unwrap_or(false)
        })
        .collect();
    dirs.sort(); // timestamp-named → chronological
    if dirs.len() > keep {
        for d in &dirs[..dirs.len() - keep] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
/// UTC `YYYY-MM-DD_HH-MM-SS` from the system clock (Hinnant civil-from-days).
fn backup_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
}

/// Build the search/backlinks cache off the hot path. We let the frontend's
/// first journal load grab the graph lock first, then warm in the background so
/// the first search is instant instead of re-parsing the whole tree.
fn warm_cache_async(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // Brief delay so the first journal paint (which only needs a few pages)
        // grabs the lock first; then build the whole-graph cache in the
        // background so the first search / query / `g j` agenda doesn't pay for
        // parsing every file synchronously under the lock.
        std::thread::sleep(std::time::Duration::from_millis(250));
        let state: State<'_, AppState> = app.state();
        let guard = state.graph.read().unwrap();
        if let Some(g) = guard.as_ref() {
            g.warm_cache();
        }
    });
}

fn with_graph<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Graph) -> Result<T, String>,
) -> Result<T, String> {
    // Clone the Arc under a brief read lock, then run `f` with the lock released
    // so concurrent commands don't block each other.
    let graph = {
        let guard = state.graph.read().unwrap();
        guard.as_ref().ok_or("no graph loaded")?.clone()
    };
    f(&graph)
}

#[tauri::command]
fn list_pages(state: State<'_, AppState>) -> Result<Vec<PageEntry>, String> {
    with_graph(&state, |g| Ok(g.list_pages()))
}

#[tauri::command]
fn journals_desc(
    limit: usize,
    offset: usize,
    state: State<'_, AppState>,
) -> Result<Vec<PageDto>, String> {
    with_graph(&state, |g| {
        let entries = g.journals_desc();
        let mut out = Vec::new();
        for e in entries.into_iter().skip(offset).take(limit) {
            match g.load_page(&e) {
                Ok(dto) => out.push(dto),
                // A journal deleted from disk between the cache listing and this
                // load just drops out of the feed — don't fail the whole batch (and
                // don't serve a stale ghost; load_page already evicted it).
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.to_string()),
            }
        }
        Ok(out)
    })
}

#[tauri::command]
fn get_page(
    name: String,
    kind: PageKind,
    state: State<'_, AppState>,
) -> Result<Option<PageDto>, String> {
    with_graph(&state, |g| g.load_named(&name, kind).map_err(|e| e.to_string()))
}

#[tauri::command]
fn save_page(
    page: PageDto,
    base_rev: Option<String>,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    with_graph(&state, |g| {
        let res = if force.unwrap_or(false) {
            g.force_save_page(&page)
        } else {
            g.save_page(&page, base_rev.as_deref())
        };
        res.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "conflict".to_string()
            } else {
                e.to_string()
            }
        })
    })
}

#[tauri::command]
fn get_backlinks(name: String, state: State<'_, AppState>) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.backlinks(&name)))
}

#[tauri::command]
fn get_unlinked_refs(name: String, state: State<'_, AppState>) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.unlinked_refs(&name)))
}

#[tauri::command]
fn delete_page(name: String, kind: PageKind, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.delete_page(&name, kind).map_err(|e| e.to_string()))
}

#[tauri::command]
fn rename_page(old: String, new: String, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.rename_page(&old, &new).map_err(|e| e.to_string()))
}

#[tauri::command]
fn publish_html(state: State<'_, AppState>) -> Result<(String, usize), String> {
    with_graph(&state, |g| g.publish_html().map_err(|e| e.to_string()))
}

#[tauri::command]
fn run_query(query: String, state: State<'_, AppState>) -> Result<Arc<Vec<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.run_query(&query)))
}

#[tauri::command]
fn run_advanced_query(
    query: String,
    current_page: Option<String>,
    state: State<'_, AppState>,
) -> Result<tine_core::query::AdvancedResult, String> {
    with_graph(&state, |g| Ok(g.run_advanced_query(&query, current_page.as_deref())))
}

#[tauri::command]
fn query_facets(state: State<'_, AppState>) -> Result<Vec<(String, Vec<String>)>, String> {
    with_graph(&state, |g| Ok(g.property_facets()))
}

#[tauri::command]
fn page_aliases(state: State<'_, AppState>) -> Result<Vec<(String, String)>, String> {
    with_graph(&state, |g| Ok(g.page_aliases()))
}

#[tauri::command]
fn set_favorites(names: Vec<String>, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.set_favorites(&names).map_err(|e| e.to_string()))
}

#[tauri::command]
fn set_preferred_workflow(workflow: String, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_preferred_workflow(&workflow).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn set_default_journal_template(
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.set_default_journal_template(name.as_deref()).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn set_start_of_week(n: u32, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.set_start_of_week(n).map_err(|e| e.to_string()))
}

/// Set the graph's `:preferred-format` for new pages/journals ("md" or "org").
#[tauri::command]
fn set_preferred_format(format: String, state: State<'_, AppState>) -> Result<(), String> {
    let fmt = if format.eq_ignore_ascii_case("org") {
        tine_core::model::Format::Org
    } else {
        tine_core::model::Format::Md
    };
    with_graph(&state, |g| g.set_preferred_format(fmt).map_err(|e| e.to_string()))
}

/// Set the graph's `:journal/page-title-format` (journal display-title format,
/// e.g. "MMM do, yyyy"). Display-only — does not rename journal files.
#[tauri::command]
fn set_journal_title_format(format: String, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.set_journal_page_title_format(&format).map_err(|e| e.to_string()))
}

#[tauri::command]
fn read_custom_css(state: State<'_, AppState>) -> Result<String, String> {
    with_graph(&state, |g| Ok(g.custom_css()))
}

/// Open a web/mail URL in the user's default external application. Scheme-gated
/// to http(s)/mailto; the URL is passed as a single argument (no shell), so it
/// can't inject commands.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")) {
        return Err("unsupported url scheme".into());
    }
    #[cfg(target_os = "linux")]
    let prog = "xdg-open";
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(target_os = "windows")]
    let prog = "explorer";
    std::process::Command::new(prog).arg(&url).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn search(query: String, limit: usize, state: State<'_, AppState>) -> Result<Vec<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.search(&query, limit)))
}

#[tauri::command]
fn quick_switch(
    query: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<PageEntry>, String> {
    with_graph(&state, |g| Ok(g.quick_switch(&query, limit)))
}

#[tauri::command]
fn list_templates(state: State<'_, AppState>) -> Result<Vec<tine_core::model::TemplateDto>, String> {
    with_graph(&state, |g| Ok(g.templates()))
}

#[tauri::command]
fn journal_content_days(state: State<'_, AppState>) -> Result<Vec<i64>, String> {
    with_graph(&state, |g| Ok(g.journal_content_days()))
}

#[tauri::command]
fn resolve_block(uuid: String, state: State<'_, AppState>) -> Result<Option<RefGroup>, String> {
    with_graph(&state, |g| Ok(g.resolve_block(&uuid)))
}

#[tauri::command]
fn resolve_blocks(uuids: Vec<String>, state: State<'_, AppState>) -> Result<Vec<Option<RefGroup>>, String> {
    with_graph(&state, |g| Ok(g.resolve_blocks(&uuids)))
}

#[tauri::command]
fn read_asset(name: String, state: State<'_, AppState>) -> Result<tauri::ipc::Response, String> {
    // Return RAW bytes (not a JSON number[]), so a multi-MB PDF/image isn't
    // serialized element-by-element and re-parsed on the JS side — the frontend
    // receives an ArrayBuffer directly.
    with_graph(&state, |g| {
        g.read_asset(&name).map(tauri::ipc::Response::new).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn import_asset(
    path: String,
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    with_graph(&state, |g| {
        g.import_asset(std::path::Path::new(&path), name.as_deref()).map_err(|e| e.to_string())
    })
}

/// Open a graph asset (by its `assets/`-relative name) in the OS default app,
/// e.g. a video/audio file in the system player. Path-gated to the assets dir
/// (canonicalized) so a crafted name can't open a file outside the graph.
#[tauri::command]
fn open_asset(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let target = with_graph(&state, |g| {
        let assets = g.assets_path();
        let canon_assets = assets.canonicalize().map_err(|e| e.to_string())?;
        let canon = assets.join(&name).canonicalize().map_err(|e| e.to_string())?;
        if !canon.starts_with(&canon_assets) {
            return Err("asset path escapes assets dir".to_string());
        }
        Ok(canon)
    })?;
    #[cfg(target_os = "linux")]
    let prog = "xdg-open";
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(target_os = "windows")]
    let prog = "explorer";
    std::process::Command::new(prog).arg(&target).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Orphaned `assets/` files (no block references them) for the cleanup UI.
#[tauri::command]
fn list_orphan_assets(
    state: State<'_, AppState>,
) -> Result<Vec<tine_core::model::AssetInfo>, String> {
    with_graph(&state, |g| Ok(g.orphan_assets()))
}

/// Move an orphaned asset to the recoverable trash.
#[tauri::command]
fn trash_asset(name: String, state: State<'_, AppState>) -> Result<(), String> {
    with_graph(&state, |g| g.trash_asset(&name).map_err(|e| e.to_string()))
}

#[tauri::command]
fn save_asset(name: String, bytes: Vec<u8>, state: State<'_, AppState>) -> Result<String, String> {
    with_graph(&state, |g| g.save_asset(&name, &bytes).map_err(|e| e.to_string()))
}

#[tauri::command]
fn read_highlights(
    pdf: String,
    state: State<'_, AppState>,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    with_graph(&state, |g| Ok(g.read_highlights(&pdf)))
}

#[tauri::command]
fn write_highlights(
    pdf: String,
    label: String,
    highlights: Vec<tine_core::pdf::Highlight>,
    base_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    with_graph(&state, |g| {
        g.write_highlights(&pdf, &label, &highlights, &base_ids).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn save_pdf_area_image(
    pdf: String,
    page: i64,
    id: String,
    stamp: i64,
    bytes: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    with_graph(&state, |g| {
        g.write_pdf_area_image(&pdf, page, &id, stamp, &bytes).map_err(|e| e.to_string())
    })
}

/// Show + focus the always-on-top quick-capture mini window (created hidden at
/// startup). Each show resets it to the small base size and anchors it near the
/// top of the screen so the frontend can grow it downward (multiple blocks, an
/// autocomplete popup, the date picker) without running off the bottom edge.
/// No-op if the window is missing.
fn show_capture(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("capture") {
        let _ = w.set_size(tauri::LogicalSize::new(600.0, 92.0));
        if let Ok(Some(mon)) = w.current_monitor() {
            let size = mon.size().to_logical::<f64>(mon.scale_factor());
            let x = ((size.width - 600.0) / 2.0).max(0.0);
            let y = size.height * 0.18;
            let _ = w.set_position(tauri::LogicalPosition::new(x, y));
        } else {
            let _ = w.center();
        }
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn main() {
    // AppImages bundle their own libwayland-client.so; on a Wayland session it can
    // mismatch the host compositor and abort WebKitGTK's EGL init ("Could not
    // create default EGL display: EGL_BAD_PARAMETER"). Self-heal by re-exec'ing
    // ONCE with the host's libwayland-client preloaded — so users never have to
    // set LD_PRELOAD by hand. Guarded to the actual problem case: only inside an
    // AppImage (`APPIMAGE` set), only on Wayland, only once (`TINE_WL_PRELOADED`),
    // only if a host lib is found. No effect on the raw binary / deb / rpm.
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("TINE_WL_PRELOADED").is_none()
        && !std::env::var("LD_PRELOAD")
            .unwrap_or_default()
            .contains("libwayland-client")
    {
        // Common host locations across distros (Debian/Ubuntu, Fedora/openSUSE, Arch).
        const CANDIDATES: &[&str] = &[
            "/usr/lib/x86_64-linux-gnu/libwayland-client.so.0",
            "/usr/lib64/libwayland-client.so.0",
            "/usr/lib/libwayland-client.so.0",
            "/lib/x86_64-linux-gnu/libwayland-client.so.0",
        ];
        if let (Some(lib), Ok(exe)) = (
            CANDIDATES.iter().find(|p| std::path::Path::new(p).exists()),
            std::env::current_exe(),
        ) {
            use std::os::unix::process::CommandExt;
            let existing = std::env::var("LD_PRELOAD").unwrap_or_default();
            let preload = if existing.is_empty() {
                lib.to_string()
            } else {
                format!("{lib}:{existing}")
            };
            // `exec` only returns on failure; on success it replaces this process
            // (same PID, env + bundled LD_LIBRARY_PATH inherited, host lib preloaded).
            let err = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .env("LD_PRELOAD", preload)
                .env("TINE_WL_PRELOADED", "1")
                .exec();
            eprintln!("tine: Wayland libwayland-client preload re-exec failed ({err}); continuing");
        }
    }

    // GPU/DMABUF rendering is ON by default (smoother scrolling — that's the point
    // of Tine). On the rare GPU/compositor combo where WebKitGTK's DMABUF renderer
    // aborts ("Could not create default EGL display: EGL_BAD_PARAMETER"), set
    // TINE_GPU=0 to fall back to software compositing. Linux-only.
    #[cfg(target_os = "linux")]
    if std::env::var("TINE_GPU").as_deref() == Ok("0")
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        // MUST be the first plugin. A second launch (e.g. the DE hotkey running
        // `tine --capture`) doesn't start a new process — this fires in the
        // already-running instance with the new argv. `--capture` pops the
        // capture window; a plain re-launch just surfaces the main window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.iter().any(|a| a == "--capture") {
                show_capture(app);
            } else if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        // Remember window size/position/maximized across launches (Wayland
        // compositors don't restore this per-app). Exclude FULLSCREEN so it
        // doesn't conflict with focus mode, which is intentionally not persisted.
        // The capture window is centered on demand, so keep it off the denylist.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .with_denylist(&["capture"])
                .build(),
        )
        // The hidden capture window would otherwise keep the process alive after
        // the main window is closed (Tauri exits only when ALL windows are gone).
        // So when `main` is destroyed, exit explicitly. The JS close handler has
        // already flushed pending edits by the time it calls destroy().
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::Destroyed = event {
                    window.app_handle().exit(0);
                }
            }
        })
        .manage(AppState { graph: RwLock::new(None), watch_ctl: Mutex::new(None) })
        .setup(|app| {
            // Eagerly open the graph if one was configured at startup.
            if let Some(root) = resolve_root("") {
                let g = Graph::open(&root);
                let meta = g.meta();
                let jdir = g.journals_path();
                let pdir = g.pages_path();
                let count_md = |d: &std::path::Path| {
                    std::fs::read_dir(d)
                        .map(|rd| {
                            rd.flatten()
                                .filter(|e| {
                                    e.path().extension().and_then(|x| x.to_str()) == Some("md")
                                })
                                .count()
                        })
                        .ok()
                };
                eprintln!("[tine] graph root: {}", meta.root);
                eprintln!(
                    "[tine] journals dir: {} (exists={}, .md files={:?})",
                    jdir.display(),
                    jdir.is_dir(),
                    count_md(&jdir)
                );
                eprintln!(
                    "[tine] pages dir: {} (exists={}, .md files={:?})",
                    pdir.display(),
                    pdir.is_dir(),
                    count_md(&pdir)
                );
                eprintln!(
                    "[tine] journals recognized as dates: {} | total page entries: {}",
                    g.journals_desc().len(),
                    g.list_pages().len()
                );
                if let Ok(rd) = std::fs::read_dir(&jdir) {
                    let sample: Vec<String> = rd
                        .flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .filter(|n| n.ends_with(".md"))
                        .take(3)
                        .collect();
                    eprintln!("[tine] sample journal files: {sample:?}");
                }
                let state: State<'_, AppState> = app.state();
                *state.graph.write().unwrap() = Some(Arc::new(g));
                // Warm the whole-graph cache in the background. Without this the
                // startup (TINE_GRAPH / argv) path never warms — only the
                // `load_graph` command did — so the user's first `g j` (whose
                // agenda query touches the whole graph) paid to parse every file
                // synchronously. First nav slow, second fine; this fixes it.
                warm_cache_async(app.handle().clone());
            } else {
                eprintln!("[tine] NO graph root resolved — set TINE_GRAPH=/path/to/graph");
            }
            // Watch for external changes (reads whichever graph is current).
            start_watcher(app.handle().clone());
            // Cold start via `tine --capture` (app wasn't already running): pop
            // the capture window once we're up (the main window loads too).
            if std::env::args().any(|a| a == "--capture") {
                show_capture(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_graph,
            list_pages,
            journals_desc,
            get_page,
            save_page,
            get_backlinks,
            get_unlinked_refs,
            delete_page,
            rename_page,
            publish_html,
            run_query,
            run_advanced_query,
            query_facets,
            page_aliases,
            set_favorites,
            set_preferred_workflow,
            set_preferred_format,
            set_journal_title_format,
            set_default_journal_template,
            set_start_of_week,
            read_custom_css,
            open_external,
            open_asset,
            list_orphan_assets,
            trash_asset,
            search,
            quick_switch,
            list_templates,
            journal_content_days,
            resolve_block,
            resolve_blocks,
            read_asset,
            import_asset,
            save_asset,
            read_highlights,
            write_highlights,
            save_pdf_area_image,
            get_backup_keep,
            set_backup_keep,
            get_capture_enter_files,
            set_capture_enter_files,
            get_watch_mode,
            set_watch_mode,
            list_backups,
            restore_backup,
            load_session,
            save_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
