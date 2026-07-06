use crate::settings::{settings_path, update_settings};
use crate::state::AppState;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tauri::{Emitter, Manager, State};
use tine_core::model::PageKind;

#[derive(Clone, serde::Serialize)]
struct GraphChange {
    name: String,
    kind: PageKind,
    removed: bool,
}

/// Recursively collect every `.md`/`.org` page file under `dir` with its
/// (mtime, len) — the watcher's diff snapshot. Descends sub-directories so a page
/// in a sub-folder (#21) is reconciled like a top-level one; mirrors the core's
/// `list_md` walk: match page files by extension (the metadata read is needed for
/// mtime/len anyway), skip hidden dirs and symlinked dirs (no cycles, no escaping
/// the watched tree). Scoped to the dir passed in (journals/ or pages/).
fn collect_page_files(dir: &std::path::Path, out: &mut HashMap<PathBuf, (SystemTime, u64)>) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("md") | Some("org")
            ) {
                if let Ok(md) = e.metadata() {
                    if let Ok(m) = md.modified() {
                        out.insert(p, (m, md.len()));
                    }
                }
                continue;
            }
            let hidden = p
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with('.'))
                .unwrap_or(true);
            if !hidden && e.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                stack.push(p);
            }
        }
    }
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
    if let Ok(mut slot) = app.state::<AppState>().watch_ctl.lock() {
        *slot = Some(tx.clone());
    }
    std::thread::spawn(move || {
        // (mtime, size) per file — not mtime alone: on coarse-mtime mounts (some
        // NFS, FAT) an external write landing in the same tick Tine already recorded
        // would otherwise read as "unchanged" and never reconcile, leaving stale
        // backlinks/queries. Size catches the overwhelmingly common case (content
        // length changed) even when the clock didn't move.
        let mut snap: HashMap<PathBuf, (SystemTime, u64)> = HashMap::new();
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
                    watcher =
                        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
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
                            // Recursive so a page created/edited in a SUB-directory
                            // (#21) — or delivered there by Syncthing — wakes the
                            // reconcile. notify emulates recursion on inotify by
                            // watching subdirs; scoped to journals/ + pages/ only.
                            if w.watch(d, notify::RecursiveMode::Recursive).is_ok() {
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
                let mut current: HashMap<PathBuf, (SystemTime, u64)> = HashMap::new();
                for dir in ds {
                    collect_page_files(dir, &mut current);
                }
                if !baseline {
                    snap = current;
                    baseline = true; // first scan establishes the baseline; emit nothing
                } else {
                    let mut changes: Vec<GraphChange> = Vec::new();
                    // A sync-tool conflict copy appearing/vanishing isn't a page
                    // change (it's never cached), but the conflicts panel must
                    // refresh — track it and emit `conflicts-changed` once.
                    let mut conflicts_dirty = false;
                    for (p, m) in &current {
                        if snap.get(p) != Some(m) {
                            if tine_core::model::path_is_sync_conflict(p) {
                                conflicts_dirty = true;
                            } else if let Some(en) = graph.sync_file(p) {
                                changes.push(GraphChange {
                                    name: en.name,
                                    kind: en.kind,
                                    removed: false,
                                });
                            }
                        }
                    }
                    for p in snap.keys() {
                        if !current.contains_key(p) {
                            if tine_core::model::path_is_sync_conflict(p) {
                                conflicts_dirty = true;
                            } else if let Some(en) = graph.forget_file(p) {
                                changes.push(GraphChange {
                                    name: en.name,
                                    kind: en.kind,
                                    removed: true,
                                });
                            }
                        }
                    }
                    snap = current;
                    for c in changes {
                        let _ = app.emit("graph-changed", c);
                    }
                    if conflicts_dirty {
                        let _ = app.emit("conflicts-changed", ());
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

        let mut out: HashMap<PathBuf, (SystemTime, u64)> = HashMap::new();
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
}
