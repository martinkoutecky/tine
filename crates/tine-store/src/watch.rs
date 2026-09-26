//! Per-store file observation. The writer mutex orders reconciliation with
//! transactions; the subscription only sees completed publications.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use notify::Watcher;

use crate::model::Graph;
use crate::store::{
    journal_ids_from_entries, ChangeFeed, ChangeKind, ConfigState, Day, FileId, FileRev, LoadError,
    LoadState, LoadStatus, Origin, PageId, WatchMode,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    identity: u128,
    changed: i128,
    rev: Option<FileRev>,
}

pub(crate) type RestoreBaseline = HashMap<PathBuf, Stamp>;

fn stamp_metadata(path: &Path) -> Option<Stamp> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    #[cfg(unix)]
    let (identity, changed) = {
        use std::os::unix::fs::MetadataExt;
        (
            ((metadata.dev() as u128) << 64) | metadata.ino() as u128,
            metadata.ctime() as i128 * 1_000_000_000 + metadata.ctime_nsec() as i128,
        )
    };
    #[cfg(windows)]
    let (identity, changed) = {
        use std::os::windows::fs::MetadataExt;
        (
            metadata.creation_time() as u128,
            metadata.last_write_time() as i128,
        )
    };
    #[cfg(not(any(unix, windows)))]
    let (identity, changed) = (
        metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos()),
        0,
    );
    Some(Stamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
        identity,
        changed,
        rev: None,
    })
}

fn stamp(path: &Path) -> Option<Stamp> {
    let mut value = stamp_metadata(path)?;
    value.rev = FileRev::from_file(path).ok();
    Some(value)
}

fn collect_dir(dir: &Path, files: &mut HashMap<PathBuf, Stamp>) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if matches!(
                path.extension().and_then(|part| part.to_str()),
                Some("md" | "org")
            ) {
                if kind.is_file() {
                    if let Some(value) = stamp_metadata(&path) {
                        files.insert(path, value);
                    }
                }
            } else if kind.is_dir()
                && !path
                    .file_name()
                    .and_then(|part| part.to_str())
                    .is_none_or(|name| name.starts_with('.'))
            {
                stack.push(path);
            }
        }
    }
}

fn collect(dirs: &[PathBuf; 2]) -> HashMap<PathBuf, Stamp> {
    let mut files = HashMap::new();
    for dir in dirs {
        collect_dir(dir, &mut files);
    }
    files
}

fn collect_with_revs(dirs: &[PathBuf; 2]) -> HashMap<PathBuf, Stamp> {
    let mut files = collect(dirs);
    for (path, value) in &mut files {
        value.rev = FileRev::from_file(path).ok();
    }
    files
}

fn collect_restore(core: &Core) -> RestoreBaseline {
    let mut files = collect_with_revs(&core.dirs.read().unwrap());
    let mut stack = vec![core.graph.assets_path()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("edn")
            {
                if let Some(value) = stamp(&path) {
                    files.insert(path, value);
                }
            }
        }
    }
    let config = core.graph.root.join("logseq/config.edn");
    if let Some(value) = stamp(&config) {
        files.insert(config, value);
    }
    files
}

fn atomic_temp(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|part| part.to_str()) else {
        return false;
    };
    let Some(stem) = name.strip_suffix(".tmp") else {
        return false;
    };
    let stem = stem.strip_suffix(".new").unwrap_or(stem);
    let Some((before_seq, seq)) = stem.rsplit_once('.') else {
        return false;
    };
    let Some((page, pid)) = before_seq.rsplit_once('.') else {
        return false;
    };
    page.starts_with('.')
        && matches!(
            Path::new(page).extension().and_then(|ext| ext.to_str()),
            Some("md" | "org")
        )
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && seq.bytes().all(|byte| byte.is_ascii_digit())
}

fn incremental_paths(event: &notify::Event) -> Option<Vec<PathBuf>> {
    use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};
    if !matches!(
        event.kind,
        EventKind::Create(CreateKind::File)
            | EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Metadata(_))
            | EventKind::Modify(ModifyKind::Name(
                RenameMode::From | RenameMode::To | RenameMode::Both
            ))
            | EventKind::Remove(RemoveKind::File)
    ) || event.paths.is_empty()
    {
        return None;
    }
    if event.paths.iter().any(|path| {
        (!matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("md" | "org")
        ) || path.is_dir())
            && !atomic_temp(path)
    }) {
        return None;
    }
    Some(
        event
            .paths
            .iter()
            .filter(|path| {
                matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("md" | "org")
                ) && !path.is_dir()
            })
            .cloned()
            .collect(),
    )
}

#[derive(Default)]
struct Pending {
    paths: HashSet<PathBuf>,
    full: bool,
}

impl Pending {
    fn add(&mut self, event: notify::Result<notify::Event>, dirs: &[PathBuf; 2]) {
        let Ok(event) = event else {
            self.full = true;
            return;
        };
        if event.need_rescan() {
            self.full |= event.paths.is_empty()
                || event
                    .paths
                    .iter()
                    .any(|path| dirs.iter().any(|dir| path.starts_with(dir)));
            return;
        }
        if let Some(paths) = incremental_paths(&event) {
            self.paths.extend(
                paths
                    .into_iter()
                    .filter(|path| dirs.iter().any(|dir| path.starts_with(dir))),
            );
        } else if event.paths.is_empty()
            || event
                .paths
                .iter()
                .any(|path| dirs.iter().any(|dir| path.starts_with(dir)))
        {
            self.full = true;
        }
    }
}

pub(crate) struct Core {
    graph: Arc<Graph>,
    writer: Arc<Mutex<()>>,
    load: Arc<LoadState>,
    changes: Arc<ChangeFeed>,
    journal_ids: Arc<Mutex<HashMap<Day, PageId>>>,
    config: Arc<RwLock<ConfigState>>,
    dirs: RwLock<[PathBuf; 2]>,
    snapshot: Mutex<HashMap<PathBuf, Stamp>>,
    config_stamp: Mutex<Option<Stamp>>,
    closed: AtomicBool,
    #[cfg(test)]
    pub(crate) recovery_reconcile_pause: Mutex<Option<crate::store::TestPause>>,
}

impl Core {
    pub(crate) fn fill_revs(&self) {
        for (path, value) in self.snapshot.lock().unwrap().iter_mut() {
            if let Some(now) = stamp_metadata(path) {
                if now.modified == value.modified
                    && now.len == value.len
                    && now.identity == value.identity
                    && now.changed == value.changed
                {
                    value.rev = FileRev::from_file(path).ok();
                }
            }
        }
    }
    fn file_id(&self, path: &Path) -> Option<FileId> {
        if let Ok(rel) = path.strip_prefix(&self.graph.assets_path()) {
            return Some(FileId::from(format!(
                "assets/{}",
                rel.to_string_lossy().replace('\\', "/")
            )));
        }
        self.graph.ensure_write_target(path).ok()?;
        let rel = path.strip_prefix(&self.graph.root).ok()?;
        Some(FileId::from(rel.to_string_lossy().replace('\\', "/")))
    }

    fn ready(&self) -> bool {
        matches!(*self.load.status.lock().unwrap(), LoadStatus::Ready)
    }

    fn reconcile(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
    ) -> Result<(), LoadError> {
        let _writer = self.writer.lock().unwrap();
        self.reconcile_locked(paths, include_config, scan_semantics)
    }

    // Caller holds writer through reconciliation and any recovery publication.
    fn reconcile_locked(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
    ) -> Result<(), LoadError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(LoadError::Closed);
        }
        if !self.graph.root.is_dir() {
            return Err(LoadError::Failed {
                reason: "graph root is unavailable".into(),
            });
        }
        let mut config_changed = false;
        if include_config {
            let path = self.graph.root.join("logseq/config.edn");
            let current = stamp(&path);
            let mut previous = self.config_stamp.lock().unwrap();
            if previous.as_ref().and_then(|value| value.rev.as_ref())
                != current.as_ref().and_then(|value| value.rev.as_ref())
            {
                self.read_config(&path)?;
                config_changed = true;
            }
            *previous = current;
        }
        let dirs = self.dirs.read().unwrap().clone();
        let mut snapshot = self.snapshot.lock().unwrap();
        let mut now = paths.map_or_else(
            || collect(&dirs),
            |paths| {
                paths
                    .iter()
                    .filter(|path| self.graph.ensure_write_target(path).is_ok())
                    .filter_map(|path| stamp(path).map(|value| (path.clone(), value)))
                    .collect()
            },
        );
        let names: HashSet<PathBuf> = if let Some(paths) = paths {
            paths.clone()
        } else {
            now.keys().chain(snapshot.keys()).cloned().collect()
        };
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort();
        let mut files = Vec::new();
        let mut pages = Vec::new();
        for path in names {
            let before = snapshot.get(&path);
            if paths.is_none() {
                if let (Some(old), Some(new)) = (before, now.get_mut(&path)) {
                    let same = old.modified == new.modified
                        && old.len == new.len
                        && (scan_semantics
                            || (old.identity == new.identity && old.changed == new.changed));
                    if same {
                        new.rev = old.rev.clone();
                        continue;
                    }
                    new.rev = FileRev::from_file(&path).ok();
                } else if let Some(new) = now.get_mut(&path) {
                    new.rev = FileRev::from_file(&path).ok();
                }
            }
            let after = now.get(&path);
            let kind = match (before, after) {
                (None, Some(_)) => Some(ChangeKind::Created),
                (Some(_), None) => Some(ChangeKind::Removed),
                (Some(a), Some(b)) if a.rev != b.rev => Some(ChangeKind::Modified),
                (Some(a), Some(b)) if a.modified != b.modified => Some(ChangeKind::Touched),
                _ => None,
            };
            let Some(kind) = kind else {
                continue;
            };
            let Some(id) = self.file_id(&path) else {
                continue;
            };
            if tine_core::model::path_is_sync_conflict(&path) {
                // Conflict copies are in the file feed so the window adapter can
                // refresh the conflicts panel, but never enter the page cache.
            } else if matches!(kind, ChangeKind::Removed) {
                if let Some(entry) = self.graph.forget_file_internal(&path) {
                    pages.push((id.clone(), entry.kind, entry.name));
                }
            } else if matches!(kind, ChangeKind::Touched) {
                self.graph
                    .observe_page_mtime(&path, after.and_then(|value| value.modified));
            } else {
                if let Some(entry) = self.graph.sync_file_internal(&path) {
                    pages.push((id.clone(), entry.kind, entry.name));
                }
            }
            files.push((id, kind, after.and_then(|value| value.rev.clone())));
        }
        if paths.is_none() {
            *snapshot = now;
        } else {
            for path in paths.unwrap() {
                if let Some(value) = now.get(path) {
                    snapshot.insert(path.clone(), value.clone());
                } else {
                    snapshot.remove(path);
                }
            }
        }
        drop(snapshot);
        if !files.is_empty() || config_changed {
            *self.journal_ids.lock().unwrap() =
                journal_ids_from_entries(&self.graph, self.graph.list_pages_shared().as_ref());
            self.changes
                .publish(Origin::External, files, config_changed, pages);
        }
        Ok(())
    }

    fn read_config(&self, path: &Path) -> Result<(), LoadError> {
        let (config, problem) = match fs::read_to_string(path) {
            Ok(value) => (tine_core::config::Config::parse(&value), None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (tine_core::config::Config::default(), None)
            }
            Err(error) => (tine_core::config::Config::default(), Some(error.into())),
        };
        self.graph
            .reload_config(config.clone())
            .map_err(|error| LoadError::Failed {
                reason: error.to_string(),
            })?;
        *self.dirs.write().unwrap() = [self.graph.journals_path(), self.graph.pages_path()];
        *self.config.write().unwrap() = ConfigState {
            config: Arc::new(config),
            problem,
        };
        Ok(())
    }

    fn note_own(&self, ids: &[FileId]) {
        let mut snapshot = self.snapshot.lock().unwrap();
        for id in ids {
            let path = self.graph.root.join(id.as_str());
            if let Some(value) = stamp(&path) {
                snapshot.insert(path.clone(), value);
            } else {
                snapshot.remove(&path);
            }
            if id.as_str() == "logseq/config.edn" {
                let _ = self.read_config(&path);
                *self.config_stamp.lock().unwrap() = stamp(&path);
            }
        }
    }
}

pub(crate) struct WatchHandle {
    core: Arc<Core>,
    mode: Arc<Mutex<WatchMode>>,
    wake: Sender<()>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl WatchHandle {
    pub(crate) fn core_for_load(&self) -> Arc<Core> {
        Arc::clone(&self.core)
    }

    pub(crate) fn wake_for_load(&self) -> Sender<()> {
        self.wake.clone()
    }

    pub(crate) fn start(
        graph: Arc<Graph>,
        writer: Arc<Mutex<()>>,
        load: Arc<LoadState>,
        changes: Arc<ChangeFeed>,
        journal_ids: Arc<Mutex<HashMap<Day, PageId>>>,
        config: Arc<RwLock<ConfigState>>,
        watch: WatchMode,
    ) -> Self {
        let dirs = [graph.journals_path(), graph.pages_path()];
        let snapshot = collect(&dirs);
        let config_stamp = stamp(&graph.root.join("logseq/config.edn"));
        let core = Arc::new(Core {
            graph,
            writer,
            load,
            changes,
            journal_ids,
            config,
            dirs: RwLock::new(dirs),
            snapshot: Mutex::new(snapshot),
            config_stamp: Mutex::new(config_stamp),
            closed: AtomicBool::new(false),
            #[cfg(test)]
            recovery_reconcile_pause: Mutex::new(None),
        });
        let mode = Arc::new(Mutex::new(watch));
        let (wake, rx) = mpsc::channel();
        let worker_core = Arc::clone(&core);
        let worker_mode = Arc::clone(&mode);
        let worker_wake = wake.clone();
        let thread = std::thread::spawn(move || run(worker_core, worker_mode, worker_wake, rx));
        Self {
            core,
            mode,
            wake,
            thread: Mutex::new(Some(thread)),
        }
    }

    pub(crate) fn set_mode(&self, mode: WatchMode) {
        *self.mode.lock().unwrap() = mode;
        let _ = self.wake.send(());
    }

    pub(crate) fn scan_refresh(&self) -> Result<(), LoadError> {
        let mut status = self.core.load.status.lock().unwrap();
        while matches!(*status, LoadStatus::Loading) {
            status = self.core.load.ready.wait(status).unwrap();
        }
        match &*status {
            LoadStatus::Closed => return Err(LoadError::Closed),
            LoadStatus::Failed(_) => {
                drop(status);
                if !self
                    .core
                    .graph
                    .warm_cache_cancellable(|| self.core.closed.load(Ordering::Acquire))
                {
                    return Err(LoadError::Failed {
                        reason: "graph load failed".into(),
                    });
                }
                let _writer = self.core.writer.lock().unwrap();
                let result = self.core.reconcile_locked(None, true, true);
                #[cfg(test)]
                crate::store::pause_at_hook(&self.core.recovery_reconcile_pause);
                if let Err(error) = result {
                    *self.core.load.status.lock().unwrap() =
                        LoadStatus::Failed(format!("{error:?}"));
                    return Err(error);
                }
                *self.core.load.status.lock().unwrap() = LoadStatus::Ready;
                self.core
                    .changes
                    .publish(Origin::External, Vec::new(), false, Vec::new());
                let _ = self.wake.send(());
                return Ok(());
            }
            LoadStatus::Ready => drop(status),
            LoadStatus::Loading => unreachable!(),
        }
        let result = self.core.reconcile(None, true, true);
        let _ = self.wake.send(());
        result
    }

    pub(crate) fn note_own(&self, ids: &[FileId]) {
        self.core.note_own(ids);
        let _ = self.wake.send(());
    }

    pub(crate) fn restore_baseline(&self) -> RestoreBaseline {
        collect_restore(&self.core)
    }

    pub(crate) fn publish_restore(&self, before: &RestoreBaseline) -> crate::store::GraphRev {
        let now = collect_restore(&self.core);
        let mut paths: Vec<_> = before.keys().chain(now.keys()).cloned().collect();
        paths.sort();
        paths.dedup();
        let mut files = Vec::new();
        let mut config_changed = false;
        for path in paths {
            let old = before.get(&path);
            let new = now.get(&path);
            if old.and_then(|value| value.rev.as_ref()) == new.and_then(|value| value.rev.as_ref())
            {
                continue;
            }
            let kind = match (old, new) {
                (None, Some(_)) => ChangeKind::Created,
                (Some(_), None) => ChangeKind::Removed,
                _ => ChangeKind::Modified,
            };
            if path == self.core.graph.root.join("logseq/config.edn") {
                config_changed = true;
                let _ = self.core.read_config(&path);
                *self.core.config_stamp.lock().unwrap() = stamp(&path);
            }
            if let Some(id) = self.core.file_id(&path) {
                files.push((id, kind, new.and_then(|value| value.rev.clone())));
            }
        }
        *self.core.snapshot.lock().unwrap() = collect_with_revs(&self.core.dirs.read().unwrap());
        *self.core.journal_ids.lock().unwrap() = journal_ids_from_entries(
            &self.core.graph,
            self.core.graph.list_pages_shared().as_ref(),
        );
        if files.is_empty() && !config_changed {
            self.core.changes.rev()
        } else if matches!(
            *self.core.load.status.lock().unwrap(),
            LoadStatus::Failed(_)
        ) {
            self.core.changes.rev()
        } else {
            self.core
                .changes
                .publish(Origin::Own, files, config_changed, Vec::new())
        }
    }

    pub(crate) fn stop(&self) {
        if !self.core.closed.swap(true, Ordering::AcqRel) {
            let _ = self.wake.send(());
            if let Some(thread) = self.thread.lock().unwrap().take() {
                let _ = thread.join();
            }
        }
    }
}

fn run(core: Arc<Core>, mode: Arc<Mutex<WatchMode>>, wake: Sender<()>, rx: Receiver<()>) {
    let pending = Arc::new(Mutex::new(Pending::default()));
    let mut watcher: Option<notify::RecommendedWatcher> = None;
    let mut active = None;
    let mut active_dirs: Option<[PathBuf; 2]> = None;
    while !core.closed.load(Ordering::Acquire) {
        let selected = *mode.lock().unwrap();
        let dirs = core.dirs.read().unwrap().clone();
        if active != Some(selected) || active_dirs.as_ref() != Some(&dirs) {
            watcher = None;
            active = Some(selected);
            active_dirs = Some(dirs.clone());
            if selected == WatchMode::Notify {
                let pending = Arc::clone(&pending);
                let wake = wake.clone();
                let callback_dirs = dirs.clone();
                if let Ok(mut created) = notify::recommended_watcher(move |event| {
                    pending.lock().unwrap().add(event, &callback_dirs);
                    let _ = wake.send(());
                }) {
                    let mut watched = false;
                    for dir in &dirs {
                        watched |= created.watch(dir, notify::RecursiveMode::Recursive).is_ok();
                    }
                    if watched {
                        watcher = Some(created);
                    }
                }
            }
        }
        if watcher.is_some() {
            if rx.recv().is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
            while rx.try_recv().is_ok() {}
            if core.closed.load(Ordering::Acquire) {
                break;
            }
            if !core.ready() {
                continue;
            }
            let mut pending = pending.lock().unwrap();
            let full = pending.full;
            let paths = std::mem::take(&mut pending.paths);
            pending.full = false;
            drop(pending);
            if full || !paths.is_empty() {
                let _ = core.reconcile(if full { None } else { Some(&paths) }, false, false);
            }
        } else {
            let _ = rx.recv_timeout(Duration::from_secs(3));
            if core.ready() && !core.closed.load(Ordering::Acquire) {
                let _ = core.reconcile(None, false, false);
            }
        }
    }
}
