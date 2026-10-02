//! Per-store file observation. The writer mutex orders reconciliation with
//! transactions; the subscription only sees completed publications.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use notify::Watcher;

use crate::asset_watch::{AssetObserver, AssetPending, AssetScope};
use crate::model::{Graph, SyncFileResult};
use crate::store::{
    journal_ids_from_entries, ChangeFeed, ChangeKind, ConfigState, Day, FileId, FileRev, LoadError,
    LoadState, LoadStatus, Origin, PageId, WatchBatch, WatchMode,
};

/// Boundary between "a burst of ordinary edits" and "an external revision"
/// (a VCS checkout, branch switch or first big sync; master 1229f32fb, GH
/// #337). One atomic save is at most two paths and a human-scale sync delta is
/// single digits to low tens; a checkout is typically hundreds. Above it a
/// drained batch escalates to the full stat diff, where an unchanged file
/// costs one stat instead of a hash, and the window adapter announces the
/// revision as one bulk event.
pub(crate) const BULK_CHANGE_THRESHOLD: usize = 32;

/// Directory names whose churn can never describe graph text: a repository or
/// a sync client's bookkeeping parked inside the graph (`git gc`, an index
/// lock per command, a `.stversions` sweep; master cd2d7562a). Every name is
/// outside graph text on its own (`graph_text_directory_scannable` refuses
/// dot-directories and `node_modules`); `tool_noise_dirs_never_hold_graph_text`
/// pins that, so this list can never hide a page. Matched relative to the
/// graph root, so a graph that itself lives under `.git/` is unaffected.
const TOOL_NOISE_DIRS: &[&str] = &[
    ".bzr",
    ".git",
    ".hg",
    ".jj",
    ".stfolder",
    ".stversions",
    ".svn",
    "node_modules",
];

fn path_is_tool_noise(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|name| TOOL_NOISE_DIRS.contains(&name))
        })
    })
}

/// True only when EVERY path of the event is tool noise. Never for a
/// rescan-required or pathless event, and never for a rename with one
/// ordinary side (a file moved out of `.git` must still be seen).
fn event_is_tool_noise(event: &notify::Event, root: &Path) -> bool {
    !event.need_rescan()
        && !event.paths.is_empty()
        && event
            .paths
            .iter()
            .all(|path| path_is_tool_noise(root, path))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    identity: u128,
    changed: i128,
    rev: Option<FileRev>,
}

pub(crate) type RestoreBaseline = HashMap<PathBuf, Stamp>;

pub(crate) fn stamp_metadata(path: &Path) -> Option<Stamp> {
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

fn retry_baseline(now: &mut HashMap<PathBuf, Stamp>, path: &Path, before: Option<&Stamp>) {
    if let Some(old) = before {
        let mut retry = old.clone();
        // Never treat a failed observation as unchanged on the next scan.
        retry.modified = None;
        retry.len = u64::MAX;
        now.insert(path.to_path_buf(), retry);
    } else {
        now.remove(path);
    }
}

fn directory_identity(path: &Path) -> Option<u128> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_dir() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(((metadata.dev() as u128) << 64) | metadata.ino() as u128)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Some(metadata.creation_time() as u128)
    }
    #[cfg(not(any(unix, windows)))]
    {
        metadata
            .created()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|time| time.as_nanos())
    }
}

fn collect_dir(
    root: &Path,
    dir: &Path,
    config: &tine_core::Config,
    files: &mut HashMap<PathBuf, Stamp>,
    unreadable: &mut HashMap<PathBuf, String>,
) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                unreadable.insert(directory, error.to_string());
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    unreadable.insert(directory.clone(), error.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    unreadable.insert(path, error.to_string());
                    continue;
                }
            };
            if crate::model::graph_text_watch_relevant(root, &path, config) {
                if kind.is_file() {
                    if let Some(value) = stamp_metadata(&path) {
                        files.insert(path, value);
                    }
                }
            } else if kind.is_dir()
                && crate::model::graph_text_directory_scannable(root, &path, config)
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

fn collect_with_errors(
    dirs: &[PathBuf; 1],
    config: &tine_core::Config,
) -> (HashMap<PathBuf, Stamp>, HashMap<PathBuf, String>) {
    let mut files = HashMap::new();
    let mut unreadable = HashMap::new();
    for dir in dirs {
        collect_dir(&dirs[0], dir, config, &mut files, &mut unreadable);
    }
    (files, unreadable)
}

fn collect(dirs: &[PathBuf; 1], config: &tine_core::Config) -> HashMap<PathBuf, Stamp> {
    collect_with_errors(dirs, config).0
}

fn collect_with_revs(dirs: &[PathBuf; 1], config: &tine_core::Config) -> HashMap<PathBuf, Stamp> {
    let mut files = collect(dirs, config);
    for (path, value) in &mut files {
        value.rev = FileRev::from_file(path).ok();
    }
    files
}

fn collect_restore(core: &Core) -> RestoreBaseline {
    let mut files = collect_with_revs(&core.dirs.read().unwrap(), &core.graph.current_config());
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
                if path.file_name().and_then(|name| name.to_str()) == Some(".tine-restore-recovery")
                {
                    continue;
                }
                stack.push(path);
            } else if kind.is_file() && crate::file_kind::is_asset_sidecar_path(&path) {
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
        && crate::file_kind::is_graph_text_path(Path::new(page))
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && seq.bytes().all(|byte| byte.is_ascii_digit())
}

/// The exact graph-text paths an event names, or `None` when it needs the
/// full stat diff. The event kind is a hint (storage spec §5.3): every named
/// graph-text path is reread, whether it now exists or not, so a missing path
/// is reconciled as a removal. GH #623: Tine's own atomic save produced events
/// this function refused (inotify `Access(Close(Write))` on the temp file;
/// on Windows `Create(Any)`/`Modify(Any)` on a temp file already renamed away,
/// `Remove(Any)` on the replaced page, `Modify(Any)` on its directory), so every
/// save cost a full stat diff of the graph under the writer.
/// Accepted gap: a removed DIRECTORY whose name ends in a graph-text extension
/// (Windows reports it as `Remove(Any)`) is reconciled as one missing page; its
/// children are noticed at the next full diff (focus return).
fn incremental_paths(event: &notify::Event) -> Option<Vec<PathBuf>> {
    use notify::event::{
        AccessKind, AccessMode, CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode,
    };
    match event.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write))
        | EventKind::Create(CreateKind::File | CreateKind::Any)
        | EventKind::Modify(
            ModifyKind::Data(_)
            | ModifyKind::Metadata(_)
            | ModifyKind::Any
            | ModifyKind::Name(RenameMode::From | RenameMode::To | RenameMode::Both),
        )
        | EventKind::Remove(RemoveKind::File | RemoveKind::Any) => {}
        // Opening or reading a file changes nothing.
        EventKind::Access(_) => return Some(Vec::new()),
        _ => return None,
    }
    if event.paths.is_empty() {
        return None;
    }
    let mut paths = Vec::with_capacity(event.paths.len());
    for path in &event.paths {
        // Tine's own temp file: its content reaches the graph through the
        // rename onto the page path, which is reported (and reread) itself.
        if atomic_temp(path) {
            continue;
        }
        if path.is_dir() {
            // A directory's own timestamps or attributes. Entries created,
            // removed or renamed inside it arrive as events of their own.
            if matches!(
                event.kind,
                EventKind::Modify(ModifyKind::Any | ModifyKind::Metadata(_))
            ) {
                continue;
            }
            return None;
        }
        if !crate::file_kind::is_graph_text_path(path) {
            return None;
        }
        paths.push(path.clone());
    }
    Some(paths)
}

#[derive(Default)]
struct Pending {
    paths: HashSet<PathBuf>,
    full: bool,
    /// First admitted notification of the batch now accumulating, for the
    /// latency receipt (one stamp per batch, none per path).
    first_event_at: Option<Instant>,
    /// An event named `logseq/config.edn`; the next cycle re-checks it.
    config: bool,
    /// Asset-lane events (metadata-only external asset observation).
    assets: AssetPending,
}

/// Whether `path` is the graph's `logseq/config.edn`, compared ASCII
/// case-insensitively: a case-folding volume reports the on-disk spelling
/// (`Logseq/Config.edn`) that the open path reaches. A false positive on a
/// case-sensitive volume costs one config stamp that finds it unchanged.
fn is_config_event_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut parts = relative.components().map(|part| part.as_os_str().to_str());
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(Some(dir)), Some(Some(file)), None)
            if dir.eq_ignore_ascii_case("logseq") && file.eq_ignore_ascii_case("config.edn")
    )
}

impl Pending {
    /// The callback's entry: the asset lane sees the event first, and an event
    /// wholly inside the assets directory (never graph text) skips the page
    /// lane, so an asset write no longer costs a full graph-text stat diff.
    fn add_event(
        &mut self,
        event: notify::Result<notify::Event>,
        dirs: &[PathBuf; 1],
        config: &tine_core::Config,
        assets: &AssetScope,
    ) -> bool {
        if matches!(&event, Ok(event) if event_is_tool_noise(event, &dirs[0])) {
            return false;
        }
        if self.assets.note(&event, assets) {
            self.first_event_at.get_or_insert_with(Instant::now);
            return true;
        }
        self.add(event, dirs, config)
    }

    /// Admit one notification; false when it can never change graph text
    /// (tool noise), in which case the watcher is not woken at all.
    fn add(
        &mut self,
        event: notify::Result<notify::Event>,
        dirs: &[PathBuf; 1],
        config: &tine_core::Config,
    ) -> bool {
        if matches!(&event, Ok(event) if event_is_tool_noise(event, &dirs[0])) {
            return false;
        }
        self.first_event_at.get_or_insert_with(Instant::now);
        let Ok(event) = event else {
            self.full = true;
            return true;
        };
        if event.need_rescan() {
            self.full |= event.paths.is_empty()
                || event
                    .paths
                    .iter()
                    .any(|path| dirs.iter().any(|dir| path.starts_with(dir)));
            return true;
        }
        // config.edn is not graph text, so the page filters below would drop
        // it (master contract §2); it has its own flag and costs no scan.
        let is_config = |path: &PathBuf| is_config_event_path(&dirs[0], path);
        if event.paths.iter().any(&is_config) {
            self.config = true;
            if event.paths.iter().all(&is_config) {
                return true;
            }
        }
        if let Some(paths) = incremental_paths(&event) {
            self.paths.extend(
                paths
                    .into_iter()
                    .filter(|path| crate::model::graph_text_watch_relevant(&dirs[0], path, config)),
            );
        } else if event.paths.is_empty()
            || event
                .paths
                .iter()
                .any(|path| dirs.iter().any(|dir| path.starts_with(dir)))
        {
            self.full = true;
        }
        true
    }

    /// Take the accumulated batch: its exact paths, whether it needs the full
    /// stat diff (unclassifiable events, or a burst above the bulk threshold),
    /// and its first-notification stamp.
    fn drain(&mut self) -> (HashSet<PathBuf>, bool, Option<Instant>) {
        let paths = std::mem::take(&mut self.paths);
        let full = std::mem::take(&mut self.full) || paths.len() > BULK_CHANGE_THRESHOLD;
        (paths, full, self.first_event_at.take())
    }
}

pub(crate) struct Core {
    graph: Arc<Graph>,
    writer: Arc<Mutex<()>>,
    load: Arc<LoadState>,
    changes: Arc<ChangeFeed>,
    journal_ids: Arc<Mutex<HashMap<Day, PageId>>>,
    config: Arc<RwLock<ConfigState>>,
    dirs: RwLock<[PathBuf; 1]>,
    assets: AssetObserver,
    snapshot: Mutex<HashMap<PathBuf, Stamp>>,
    config_stamp: Mutex<Option<Stamp>>,
    unreadable_dirs: Mutex<HashMap<PathBuf, String>>,
    closed: AtomicBool,
    #[cfg(test)]
    pub(crate) recovery_reconcile_pause: Mutex<Option<crate::store::TestPause>>,
    #[cfg(test)]
    pub(crate) recovery_warm_pause: Mutex<Option<crate::store::TestPause>>,
    #[cfg(test)]
    pub(crate) note_own_pause: Mutex<Option<crate::store::TestPause>>,
    #[cfg(test)]
    pub(crate) after_collect_pause: Mutex<Option<crate::store::TestPause>>,
    #[cfg(test)]
    force_mismatched_rev_once: AtomicBool,
    /// Event-driven batches that took the full stat diff (GH #623 tests).
    #[cfg(test)]
    pub(crate) event_full_diffs: std::sync::atomic::AtomicUsize,
}

impl Core {
    /// Record the live-notification state on the change feed, which tells
    /// its subscriber (see `ChangeFeed::set_watch_refusal`).
    fn set_refusal(&self, now: Option<String>, restored: bool) {
        self.changes.set_watch_refusal(now, restored);
    }

    fn path_for_id(&self, id: &FileId) -> PathBuf {
        if let Some(rel) = id.as_str().strip_prefix("assets/") {
            self.graph.assets_path().join(rel)
        } else {
            self.graph.root.join(id.as_str())
        }
    }

    fn tracks_in_snapshot(&self, path: &Path) -> bool {
        crate::file_kind::is_graph_text_path(path)
            && self
                .dirs
                .read()
                .unwrap()
                .iter()
                .any(|dir| path.starts_with(dir))
    }

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

    /// One watcher cycle; its publication carries `batch` for latency receipts.
    fn reconcile_batch(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        batch: WatchBatch,
    ) -> Result<(), LoadError> {
        let _writer = self.writer.lock().unwrap();
        self.reconcile_inner(paths, include_config, false, Some(batch))
    }

    /// One asset-lane cycle: metadata-only, publishes `Origin::External`
    /// `assets/<rel>` tuples without revisions. Writer-ordered like every
    /// publication so an own asset write and its baseline update cannot
    /// interleave with the comparison.
    fn observe_assets(&self, exact: &HashSet<PathBuf>, full: bool) {
        let _writer = self.writer.lock().unwrap();
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let files: Vec<_> = self
            .assets
            .reconcile(exact, full)
            .into_iter()
            .filter_map(|(path, kind)| Some((self.file_id(&path)?, kind, None)))
            .collect();
        if !files.is_empty() {
            self.changes
                .publish_watched(Origin::External, files, false, Vec::new(), || {}, None);
        }
    }

    // Caller holds writer through reconciliation and any recovery publication.
    fn reconcile_locked(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
    ) -> Result<(), LoadError> {
        self.reconcile_inner(paths, include_config, scan_semantics, None)
    }

    fn reconcile_inner(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
        batch: Option<WatchBatch>,
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
        let mut config_file = None;
        if include_config {
            let path = self.graph.root.join("logseq/config.edn");
            let mut current = stamp(&path);
            let mut previous = self.config_stamp.lock().unwrap();
            // A stamp without a hash means the file vanished (or failed to
            // read) between its metadata and its bytes: a sync delivery or
            // external editor removing it mid-cycle. Look again so a removal
            // reads as one; otherwise it passed as a hashless "modification"
            // and the later real removal compared hashless to absent, unseen.
            if (current.is_none() && previous.is_some())
                || current.as_ref().is_some_and(|value| value.rev.is_none())
            {
                current = stamp(&path);
            }
            // Byte-identity gate: taking in a config discards every parsed
            // page, so only a changed revision is read.
            let moved = previous.as_ref().and_then(|value| value.rev.as_ref())
                != current.as_ref().and_then(|value| value.rev.as_ref());
            match moved.then(|| self.read_config(&path)) {
                None => *previous = current,
                Some(Ok(())) => {
                    config_changed = true;
                    let kind = match (previous.as_ref(), current.as_ref()) {
                        (None, Some(_)) => ChangeKind::Created,
                        (Some(_), None) => ChangeKind::Removed,
                        _ => ChangeKind::Modified,
                    };
                    config_file = Some((
                        FileId::from("logseq/config.edn".to_owned()),
                        kind,
                        current.as_ref().and_then(|value| value.rev.clone()),
                    ));
                    *previous = current;
                }
                Some(Err(error)) if scan_semantics => return Err(error),
                // Refusal (sync delivery / external-editor race): the delivered
                // config names a page or journal directory that escapes the
                // graph. A watcher cycle keeps serving the last good config,
                // keeps the old stamp so a later cycle re-checks, and still
                // observes page files: a bad config must not blind the
                // watcher. Contract `docs/contracts/config-live-reload.md` §5.
                Some(Err(_)) => {}
            }
        }
        // A changed config can change which files are graph text (`:hidden`,
        // page and journal directories), so it takes a full scan.
        let paths = if config_changed { None } else { paths };
        let dirs = self.dirs.read().unwrap().clone();
        let mut snapshot = self.snapshot.lock().unwrap();
        let (mut now, mut unreadable) = if let Some(paths) = paths {
            (
                paths
                    .iter()
                    .filter(|path| {
                        if path.starts_with(self.graph.assets_path()) {
                            self.graph.ensure_asset_write_target(path).is_ok()
                        } else {
                            self.graph.ensure_write_target(path).is_ok()
                        }
                    })
                    .filter_map(|path| stamp(path).map(|value| (path.clone(), value)))
                    .collect(),
                Some(self.unreadable_dirs.lock().unwrap().clone()),
            )
        } else {
            let (files, errors) = collect_with_errors(&dirs, &self.graph.current_config());
            (files, Some(errors))
        };
        #[cfg(test)]
        if snapshot.keys().any(|path| !now.contains_key(path)) {
            crate::store::pause_at_hook(&self.after_collect_pause);
        }
        if let Some(errors) = unreadable.as_ref() {
            for (path, value) in &*snapshot {
                if path
                    .ancestors()
                    .any(|ancestor| errors.contains_key(ancestor))
                {
                    now.entry(path.clone()).or_insert_with(|| value.clone());
                }
            }
        }
        let names: HashSet<PathBuf> = if let Some(paths) = paths {
            paths.clone()
        } else {
            now.keys().chain(snapshot.keys()).cloned().collect()
        };
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort();
        let mut files = Vec::new();
        if let Some(config_file) = config_file {
            files.push(config_file);
        }
        let mut pages = Vec::new();
        for path in names {
            let before = snapshot.get(&path);
            if before.is_some() && !now.contains_key(&path) {
                if let Some(value) = stamp(&path) {
                    now.insert(path.clone(), value);
                }
            }
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
                let observed_rev = after.and_then(|value| value.rev.clone());
                #[cfg(test)]
                let observed_rev = if self.force_mismatched_rev_once.swap(false, Ordering::AcqRel) {
                    Some(FileRev::from_bytes(b"injected mismatched hash"))
                } else {
                    observed_rev
                };
                let Some(expected_rev) = observed_rev.as_ref() else {
                    unreadable
                        .as_mut()
                        .unwrap()
                        .insert(path.clone(), "file hash failed".into());
                    retry_baseline(&mut now, &path, before);
                    continue;
                };
                match self.graph.sync_file_internal(&path, Some(expected_rev)) {
                    SyncFileResult::Reconciled { entry, rev } => {
                        debug_assert_eq!(&rev, expected_rev);
                        unreadable.as_mut().unwrap().remove(&path);
                        if let Some(entry) = entry {
                            pages.push((id.clone(), entry.kind, entry.name));
                        }
                    }
                    SyncFileResult::ChangedDuringRead => {
                        unreadable.as_mut().unwrap().remove(&path);
                        retry_baseline(&mut now, &path, before);
                        continue;
                    }
                    SyncFileResult::ReadFailed(error) => {
                        unreadable
                            .as_mut()
                            .unwrap()
                            .insert(path.clone(), error.to_string());
                        retry_baseline(&mut now, &path, before);
                        continue;
                    }
                    SyncFileResult::Excluded => {
                        unreadable.as_mut().unwrap().remove(&path);
                    }
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
        let unreadable_changed = if let Some(errors) = unreadable {
            let mut previous = self.unreadable_dirs.lock().unwrap();
            let changed = *previous != errors;
            if changed {
                self.graph
                    .replace_unreadable_walk_errors(&previous, &errors);
                *previous = errors;
            }
            changed
        } else {
            false
        };
        drop(snapshot);
        if !files.is_empty() || config_changed || unreadable_changed {
            self.changes.publish_watched(
                Origin::External,
                files,
                config_changed,
                pages,
                || {},
                batch,
            );
        }
        Ok(())
    }

    fn read_config(&self, path: &Path) -> Result<(), LoadError> {
        let (config, problem) = match crate::model::read_parse_input(path) {
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
        *self.dirs.write().unwrap() = [self.graph.root.clone()];
        *self.config.write().unwrap() = ConfigState {
            config: Arc::new(config),
            problem,
            assets_directory_name: self
                .graph
                .assets_path()
                .file_name()
                .and_then(|part| part.to_str())
                .unwrap_or("dir")
                .to_owned(),
        };
        Ok(())
    }

    fn note_own(&self, files: &[(FileId, Option<FileRev>)]) -> HashSet<PathBuf> {
        #[cfg(test)]
        crate::store::pause_at_hook(&self.note_own_pause);
        let mut snapshot = self.snapshot.lock().unwrap();
        let mut raced = HashSet::new();
        for (id, expected) in files {
            let path = self.path_for_id(id);
            if id.as_str().starts_with("assets/") {
                self.assets.note_own(&path);
            }
            let current = stamp(&path);
            let tracked = self.tracks_in_snapshot(&path);
            if current.as_ref().and_then(|value| value.rev.as_ref()) != expected.as_ref() {
                // Reconciliation must compare disk with the revision just
                // published, not with the pre-operation watcher stamp.
                if let Some(rev) = expected {
                    let mut published = current
                        .clone()
                        .or_else(|| snapshot.get(&path).cloned())
                        .unwrap_or(Stamp {
                            modified: None,
                            len: 0,
                            identity: 0,
                            changed: 0,
                            rev: None,
                        });
                    published.rev = Some(rev.clone());
                    snapshot.insert(path.clone(), published);
                } else {
                    snapshot.remove(&path);
                }
                raced.insert(path);
                continue;
            }
            if tracked {
                if let Some(value) = current.clone() {
                    snapshot.insert(path.clone(), value);
                } else {
                    snapshot.remove(&path);
                }
            } else {
                snapshot.remove(&path);
            }
            if id.as_str() == "logseq/config.edn" {
                let _ = self.read_config(&path);
                *self.config_stamp.lock().unwrap() = current;
            }
        }
        raced
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
        let dirs = [graph.root.clone()];
        // The asset baseline is captured before any OS watch exists.
        let asset_scope = AssetScope::new(&graph);
        let snapshot = collect(&dirs, &graph.current_config());
        let config_stamp = stamp(&graph.root.join("logseq/config.edn"));
        let core = Arc::new(Core {
            graph,
            writer,
            load,
            changes,
            journal_ids,
            config,
            dirs: RwLock::new(dirs),
            assets: AssetObserver::new(asset_scope),
            snapshot: Mutex::new(snapshot),
            config_stamp: Mutex::new(config_stamp),
            unreadable_dirs: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            #[cfg(test)]
            recovery_reconcile_pause: Mutex::new(None),
            #[cfg(test)]
            recovery_warm_pause: Mutex::new(None),
            #[cfg(test)]
            note_own_pause: Mutex::new(None),
            #[cfg(test)]
            after_collect_pause: Mutex::new(None),
            #[cfg(test)]
            force_mismatched_rev_once: AtomicBool::new(false),
            #[cfg(test)]
            event_full_diffs: std::sync::atomic::AtomicUsize::new(0),
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
                #[cfg(test)]
                crate::store::pause_at_hook(&self.core.recovery_warm_pause);
                if !self
                    .core
                    .graph
                    .warm_cache_cancellable(|| self.core.closed.load(Ordering::Acquire))
                {
                    if self.core.closed.load(Ordering::Acquire) {
                        return Err(LoadError::Closed);
                    }
                    return Err(LoadError::Failed {
                        reason: "graph load failed".into(),
                    });
                }
                let _writer = self.core.writer.lock().unwrap();
                let result = self.core.reconcile_locked(None, true, true);
                #[cfg(test)]
                crate::store::pause_at_hook(&self.core.recovery_reconcile_pause);
                if let Err(error) = result {
                    let mut status = self.core.load.status.lock().unwrap();
                    if matches!(*status, LoadStatus::Closed) {
                        return Err(LoadError::Closed);
                    }
                    *status = LoadStatus::Failed(format!("{error:?}"));
                    return Err(error);
                }
                self.core.changes.publish_with(
                    Origin::External,
                    Vec::new(),
                    false,
                    Vec::new(),
                    || *self.core.load.status.lock().unwrap() = LoadStatus::Ready,
                );
                self.core.load.ready.notify_all();
                let _ = self.wake.send(());
                return Ok(());
            }
            LoadStatus::Ready => drop(status),
            LoadStatus::Loading => unreachable!(),
        }
        let result = self.core.reconcile(None, true, true);
        if result.is_ok() {
            self.core.observe_assets(&HashSet::new(), true);
        }
        let _ = self.wake.send(());
        result
    }

    pub(crate) fn note_own(&self, files: &[(FileId, Option<FileRev>)]) -> HashSet<PathBuf> {
        let raced = self.core.note_own(files);
        let _ = self.wake.send(());
        raced
    }

    /// The transaction's final bytes for this path are now the asset baseline,
    /// also when it was rolled back and published nothing (no external echo).
    pub(crate) fn settle_asset(&self, path: &Path) {
        self.core.assets.note_own(path);
    }

    pub(crate) fn reconcile_raced(&self, paths: &HashSet<PathBuf>) {
        if !paths.is_empty() {
            let _ = self.core.reconcile_locked(Some(paths), true, false);
            let mut snapshot = self.core.snapshot.lock().unwrap();
            for path in paths {
                if !self.core.tracks_in_snapshot(path) {
                    snapshot.remove(path);
                }
            }
        }
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
                if id.as_str().starts_with("assets/") {
                    self.core.assets.note_own(&path);
                }
                files.push((id, kind, new.and_then(|value| value.rev.clone())));
            }
        }
        *self.core.snapshot.lock().unwrap() = collect_with_revs(
            &self.core.dirs.read().unwrap(),
            &self.core.graph.current_config(),
        );
        if files.is_empty() && !config_changed {
            self.core.changes.rev()
        } else if matches!(
            *self.core.load.status.lock().unwrap(),
            LoadStatus::Failed(_)
        ) {
            *self.core.journal_ids.lock().unwrap() = journal_ids_from_entries(
                &self.core.graph,
                self.core.graph.list_pages_shared().as_ref(),
            );
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

#[cfg(test)]
static REFUSED_ROOTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Install live notifications for the graph root, or say why the OS refused
/// (inotify's per-user watch limit, a network mount, a missing root).
fn install_watch(
    core: &Arc<Core>,
    dirs: &[PathBuf; 1],
    pending: &Arc<Mutex<Pending>>,
    wake: &Sender<()>,
) -> Result<notify::RecommendedWatcher, String> {
    #[cfg(test)]
    if REFUSED_ROOTS.lock().unwrap().contains(&dirs[0]) {
        return Err("watch refused by test".into());
    }
    let pending = Arc::clone(pending);
    let wake = wake.clone();
    let callback_dirs = dirs.clone();
    let callback_graph = Arc::clone(&core.graph);
    let callback_assets = core.assets.scope().clone();
    let mut created = notify::recommended_watcher(move |event| {
        let admitted = pending.lock().unwrap().add_event(
            event,
            &callback_dirs,
            &callback_graph.current_config(),
            &callback_assets,
        );
        if admitted {
            let _ = wake.send(());
        }
    })
    .map_err(|error| error.to_string())?;
    for dir in dirs {
        created
            .watch(dir, notify::RecursiveMode::Recursive)
            .map_err(|error| error.to_string())?;
    }
    Ok(created)
}

/// Watch the approved external assets root, which lies outside the graph so
/// the graph-root watch may not reach it (links are followed on inotify only).
/// A refusal is secondary: only that root is affected. It is retried every
/// cycle and covered by a stat pass of the root on the same cycle, the graph's
/// own watch stays live, and no graph-level status is raised. The failure is
/// logged once per distinct message through the diagnostic line channel
/// (a fixed line, no path: I-5; it reaches the `--debug` log).
fn watch_external_assets(
    core: &Core,
    watcher: &mut notify::RecommendedWatcher,
    root: &Path,
    last_failure: &mut Option<String>,
) -> bool {
    #[cfg(test)]
    let refused = REFUSED_ROOTS.lock().unwrap().iter().any(|dir| dir == root);
    #[cfg(not(test))]
    let refused = false;
    let result = if refused {
        Err("watch refused by test".to_owned())
    } else {
        notify::Watcher::watch(watcher, root, notify::RecursiveMode::Recursive)
            .map_err(|error| error.to_string())
    };
    match result {
        Ok(()) => {
            *last_failure = None;
            // The root was unobserved until now: catch anything it missed.
            core.observe_assets(&HashSet::new(), true);
            true
        }
        Err(message) => {
            if last_failure.as_ref() != Some(&message) {
                tine_core::diag_line::diagnostic_line(
                    "Tine could not watch the external assets folder; polling it instead",
                );
                *last_failure = Some(message);
            }
            false
        }
    }
}

fn run(core: Arc<Core>, mode: Arc<Mutex<WatchMode>>, wake: Sender<()>, rx: Receiver<()>) {
    let pending = Arc::new(Mutex::new(Pending::default()));
    let mut watcher: Option<notify::RecommendedWatcher> = None;
    let mut active = None;
    let mut active_dirs: Option<[PathBuf; 1]> = None;
    let mut active_dir_ids: Option<[Option<u128>; 1]> = None;
    let mut assets_watched = false;
    let mut assets_failure: Option<String> = None;
    while !core.closed.load(Ordering::Acquire) {
        let selected = *mode.lock().unwrap();
        let dirs = core.dirs.read().unwrap().clone();
        let dir_ids = [directory_identity(&dirs[0])];
        // A refused watch is retried every cycle: polling covers the gap (I-9:
        // the refusal was reported, so it is never silent staleness).
        let retry = selected == WatchMode::Notify && watcher.is_none();
        if retry
            || active != Some(selected)
            || active_dirs.as_ref() != Some(&dirs)
            || active_dir_ids.as_ref() != Some(&dir_ids)
        {
            let retrying = retry && active == Some(selected);
            watcher = None;
            assets_watched = false;
            active = Some(selected);
            active_dirs = Some(dirs.clone());
            active_dir_ids = Some(dir_ids);
            if selected == WatchMode::Notify {
                match install_watch(&core, &dirs, &pending, &wake) {
                    Ok(created) => {
                        watcher = Some(created);
                        core.set_refusal(None, true);
                    }
                    Err(message) => core.set_refusal(Some(message), true),
                }
            } else {
                core.set_refusal(None, false);
            }
            if core.ready() && (watcher.is_some() || !retrying) {
                // A recreated root, or a watch installed after polling, can
                // already contain files (or a config.edn) it never reported.
                // Reconcile once, config included.
                let _ = core.reconcile(None, true, false);
            }
        }
        let external_assets = core
            .assets
            .scope()
            .external_root(&dirs[0])
            .map(Path::to_path_buf);
        let assets_polled = match (watcher.as_mut(), external_assets) {
            (Some(live), Some(root)) if !assets_watched => {
                assets_watched = watch_external_assets(&core, live, &root, &mut assets_failure);
                !assets_watched
            }
            _ => false,
        };
        if watcher.is_some() {
            match rx.recv_timeout(Duration::from_secs(3)) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if assets_polled && core.ready() {
                        core.observe_assets(&HashSet::new(), true);
                    }
                    continue;
                }
                Ok(()) => {}
            }
            std::thread::sleep(Duration::from_millis(200));
            while rx.try_recv().is_ok() {}
            if core.closed.load(Ordering::Acquire) {
                break;
            }
            if !core.ready() {
                continue;
            }
            let (paths, full, config, first_event_at, assets) = {
                let mut pending = pending.lock().unwrap();
                let config = std::mem::take(&mut pending.config);
                let (paths, full, first_event_at) = pending.drain();
                (paths, full, config, first_event_at, pending.assets.drain())
            };
            // A rescan or unusable event may hide a config write: re-check it.
            #[cfg(test)]
            if full {
                core.event_full_diffs.fetch_add(1, Ordering::Relaxed);
            }
            if full || config || !paths.is_empty() {
                let batch = WatchBatch {
                    first_event_at,
                    reconcile_started: Instant::now(),
                    poll: false,
                    full_diff: full,
                    event_paths: paths.len(),
                };
                let _ = core.reconcile_batch(
                    if full { None } else { Some(&paths) },
                    full || config,
                    batch,
                );
            }
            // A rescan/unusable event hides asset changes too: scan them.
            if full || assets.1 || assets_polled || !assets.0.is_empty() {
                core.observe_assets(&assets.0, full || assets.1 || assets_polled);
            }
        } else {
            // Poll mode has no event paths: every cycle re-checks the config
            // (one stat and one hash of a small file beside the full stat scan).
            let _ = rx.recv_timeout(Duration::from_secs(3));
            if core.ready() && !core.closed.load(Ordering::Acquire) {
                let batch = WatchBatch {
                    first_event_at: None,
                    reconcile_started: Instant::now(),
                    poll: true,
                    full_diff: true,
                    event_paths: 0,
                };
                let _ = core.reconcile_batch(None, true, batch);
                core.observe_assets(&HashSet::new(), true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{OpenOptions, Store};

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tine-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        root
    }

    fn modify(paths: Vec<PathBuf>) -> notify::Event {
        notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths,
            attrs: Default::default(),
        }
    }

    /// Receive until `until` holds for a received change, or panic after 10 s.
    fn wait_for(
        subscription: &crate::store::Subscription,
        until: impl Fn(&crate::store::Change) -> bool,
    ) -> crate::store::Change {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match subscription.try_recv().unwrap() {
                Some(change) if until(&change) => return change,
                Some(_) => {}
                None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                None => panic!("the watcher never published the expected change"),
            }
        }
    }

    /// og-J2: an event wholly inside the assets directory belongs to the asset
    /// lane only. It used to fall through to "unknown path under the graph" and
    /// cost a full graph-text stat diff per asset write; a page event, a move
    /// of the assets directory itself, and an unclassifiable event still reach
    /// the page lane.
    #[test]
    fn asset_events_use_the_asset_lane_and_leave_the_page_lane_alone() {
        let root = temp_root("asset-lane");
        fs::create_dir_all(root.join("assets/sub")).unwrap();
        fs::write(root.join("assets/sub/pic.png"), b"x").unwrap();
        let graph = crate::model::Graph::open(&root);
        let scope = AssetScope::new(&graph);
        let config = tine_core::Config::default();
        let dirs = [root.clone()];

        let mut pending = Pending::default();
        let inside = modify(vec![root.join("assets/sub/pic.png")]);
        assert!(pending.add_event(Ok(inside), &dirs, &config, &scope));
        assert!(
            !pending.full && pending.paths.is_empty(),
            "asset write escalated the page lane"
        );
        let (exact, full) = pending.assets.drain();
        assert!(!full && exact.contains(&root.join("assets/sub/pic.png")));

        // A page still takes the page lane, and touches no asset queue.
        let page = root.join("pages/A.md");
        fs::write(&page, "- a\n").unwrap();
        assert!(pending.add_event(Ok(modify(vec![page.clone()])), &dirs, &config, &scope));
        assert!(pending.paths.contains(&page));
        let (exact, full) = pending.assets.drain();
        assert!(exact.is_empty() && !full);

        // The assets directory itself moving: both lanes rescan.
        let mut pending = Pending::default();
        let moved = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            paths: vec![root.join("assets"), root.join("assets-old")],
            attrs: Default::default(),
        };
        assert!(pending.add_event(Ok(moved), &dirs, &config, &scope));
        assert!(pending.full);
        assert!(pending.assets.drain().1);

        // Kernel overflow and notify errors force the asset scan too.
        let mut pending = Pending::default();
        assert!(pending.add_event(Err(notify::Error::generic("boom")), &dirs, &config, &scope));
        assert!(pending.full && pending.assets.drain().1);
        fs::remove_dir_all(root).unwrap();
    }

    /// Master cd2d7562a: a repository or sync client parked inside the graph
    /// never wakes the watcher, and never escalates a full rescan; a rename
    /// out of `.git` is still seen.
    #[test]
    fn tool_noise_never_wakes_the_watcher_and_never_hides_a_page() {
        let root = temp_root("noise");
        let config = tine_core::Config::default();
        for name in TOOL_NOISE_DIRS {
            let dir = root.join(name);
            assert!(
                !crate::model::graph_text_directory_scannable(&root, &dir, &config)
                    && !crate::model::graph_text_watch_relevant(&root, &dir.join("Page.md"), &config),
                "{name} must be outside graph text on its own, or the noise filter could hide a page"
            );
            let mut pending = Pending::default();
            let admitted = pending.add(
                Ok(modify(vec![dir.join("index"), dir.join("sub/Page.md")])),
                &[root.clone()],
                &config,
            );
            assert!(
                !admitted && !pending.full && pending.paths.is_empty(),
                "{name}"
            );
            assert!(pending.first_event_at.is_none());
        }
        let page = root.join("pages/A.md");
        fs::write(&page, "- a\n").unwrap();
        let mut pending = Pending::default();
        let rename = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            paths: vec![root.join(".git/A.md"), page.clone()],
            attrs: Default::default(),
        };
        assert!(pending.add(Ok(rename), &[root.clone()], &config));
        assert!(pending.full || pending.paths.contains(&page));
        let mut overflow = Pending::default();
        assert!(overflow.add(
            Ok(modify(Vec::new()).set_flag(notify::event::Flag::Rescan)),
            &[root.clone()],
            &config
        ));
        assert!(overflow.full, "a kernel queue overflow is never noise");
        fs::remove_dir_all(root).unwrap();
    }

    /// Master 1229f32fb (GH #337): a drained batch above the bulk threshold
    /// takes the full stat diff (one stat per unchanged file instead of a hash
    /// per evented path); at the threshold it stays incremental.
    #[test]
    fn a_checkout_sized_burst_escalates_to_one_full_diff() {
        let root = temp_root("burst");
        let config = tine_core::Config::default();
        for (count, escalates) in [
            (BULK_CHANGE_THRESHOLD, false),
            (BULK_CHANGE_THRESHOLD + 1, true),
        ] {
            let mut pending = Pending::default();
            for index in 0..count {
                let page = root.join(format!("pages/P{index}.md"));
                fs::write(&page, "- checked out\n").unwrap();
                assert!(pending.add(Ok(modify(vec![page])), &[root.clone()], &config));
            }
            let (paths, full, first) = pending.drain();
            assert_eq!((paths.len(), full), (count, escalates));
            assert!(first.is_some());
            assert_eq!(
                pending.drain(),
                (HashSet::new(), false, None),
                "drain empties the batch"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// og-J2 (master runtime.rs watches per directory): a refused watch of the
    /// approved external assets root affects only that root. The graph's own
    /// watch stays live (page edits arrive by events, not the poll path), no
    /// graph-level refusal is raised, the root is stat-polled on the existing
    /// cycle, and the watch is retried until it is installed. In-scope
    /// scenario: inotify's per-user watch limit reached while installing the
    /// second watch, or an external volume without notifications.
    #[cfg(unix)]
    #[test]
    fn a_refused_external_assets_watch_never_demotes_the_graph_watch() {
        let root = std::fs::canonicalize(temp_root("refused-assets")).unwrap();
        let external = std::fs::canonicalize(temp_root("refused-assets-real")).unwrap();
        fs::write(external.join("pic.png"), b"first").unwrap();
        std::os::unix::fs::symlink(&external, root.join("assets")).unwrap();
        REFUSED_ROOTS.lock().unwrap().push(external.clone());
        let store = Store::open(
            &root,
            OpenOptions {
                approved_external_assets: Some(external.clone()),
                watch: WatchMode::Notify,
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&statuses);
        let subscription = store.subscribe();
        subscription.observe_watch_status(move |status| sink.lock().unwrap().push(status));
        std::thread::sleep(Duration::from_millis(400));

        fs::write(root.join("pages/Live.md"), "- seen by events\n").unwrap();
        let change = wait_for(&subscription, |change| {
            change
                .files
                .iter()
                .any(|(id, _, _)| id.as_str() == "pages/Live.md")
        });
        let batch = change.watch.unwrap();
        assert!(
            !batch.poll && batch.first_event_at.is_some(),
            "page edit took the poll path"
        );

        fs::write(external.join("pic.png"), b"second, longer").unwrap();
        wait_for(&subscription, |change| {
            change.origin == crate::store::Origin::External
                && change.files.iter().any(|(id, kind, _)| {
                    id.as_str() == "assets/pic.png" && *kind == ChangeKind::Modified
                })
        });

        REFUSED_ROOTS
            .lock()
            .unwrap()
            .retain(|refused| refused != &external);
        std::thread::sleep(Duration::from_millis(3500));
        fs::write(external.join("pic.png"), b"third, longer still").unwrap();
        wait_for(&subscription, |change| {
            change
                .files
                .iter()
                .any(|(id, _, _)| id.as_str() == "assets/pic.png")
        });
        assert!(
            statuses.lock().unwrap().is_empty(),
            "a secondary refusal raised a graph-level status: {:?}",
            statuses.lock().unwrap()
        );
        store.close();
    }

    /// I-9: the OS refusing live notifications degrades to polling, is
    /// reported with its reason, is retried every cycle, and the restored
    /// watch is reported too. In-scope scenario: inotify's per-user watch
    /// limit reached by a second large graph, or a network mount.
    #[test]
    fn a_refused_watch_polls_reports_and_recovers() {
        // The store watches the canonical root; on Windows the temp dir is an
        // 8.3 short path, so the refusal hook must be keyed by the canonical one.
        let root = crate::Store::canonical_root(&temp_root("refused")).unwrap();
        REFUSED_ROOTS.lock().unwrap().push(root.clone());
        let store = Store::open(
            &root,
            OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Notify,
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&statuses);
        let subscription = store.subscribe();
        subscription.observe_watch_status(move |status| sink.lock().unwrap().push(status));
        let deadline = Instant::now() + Duration::from_secs(10);
        while statuses.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            *statuses.lock().unwrap(),
            vec![Some("watch refused by test".to_owned())]
        );
        fs::write(root.join("pages/Polled.md"), "- seen by polling\n").unwrap();
        let change = wait_for(&subscription, |change| {
            change
                .files
                .iter()
                .any(|(id, _, _)| id.as_str() == "pages/Polled.md")
        });
        assert!(change
            .watch
            .is_some_and(|batch| batch.poll && batch.full_diff));
        std::thread::sleep(Duration::from_millis(3500));
        assert_eq!(
            statuses.lock().unwrap().len(),
            1,
            "a retry failing the same way stays quiet"
        );
        REFUSED_ROOTS
            .lock()
            .unwrap()
            .retain(|refused| refused != &root);
        let deadline = Instant::now() + Duration::from_secs(10);
        while statuses.lock().unwrap().len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(statuses.lock().unwrap()[1], None, "restored");
        std::thread::sleep(Duration::from_millis(150));
        fs::write(root.join("pages/Live.md"), "- seen live\n").unwrap();
        let change = wait_for(&subscription, |change| {
            change
                .files
                .iter()
                .any(|(id, _, _)| id.as_str() == "pages/Live.md")
        });
        let batch = change.watch.unwrap();
        assert!(!batch.poll && batch.first_event_at.is_some());
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn watcher_inventory_and_incremental_path_share_hidden_policy() {
        let root = std::env::temp_dir().join(format!("tine-hidden-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("archive/private")).unwrap();
        fs::create_dir_all(root.join("archive/public")).unwrap();
        let hidden = root.join("archive/private/Secret.md");
        let visible = root.join("archive/public/Visible.md");
        fs::write(&hidden, "- hidden\n").unwrap();
        fs::write(&visible, "- visible\n").unwrap();
        let config = tine_core::Config::parse(r#"{:hidden ["archive/private"]}"#);
        let files = collect(&[root.clone()], &config);
        assert!(!files.contains_key(&hidden));
        assert!(files.contains_key(&visible));
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![hidden.clone(), visible.clone()],
            attrs: Default::default(),
        };
        let mut pending = Pending::default();
        pending.add(Ok(event), &[root.clone()], &config);
        assert!(!pending.paths.contains(&hidden));
        assert!(pending.paths.contains(&visible));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_any_events_on_exact_text_files_stay_incremental() {
        use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind};

        let root = std::env::temp_dir().join(format!(
            "tine-win-any-{}-{}",
            std::process::id(),
            // Debug of SystemTime prints `{` and `:`; Windows rejects them in file names.
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pages = root.join("pages");
        fs::create_dir_all(&pages).unwrap();
        let text = pages.join("TINE版本更新提示词.md");
        let directory = pages.join("folder.md");
        fs::write(&text, "- text\n").unwrap();
        fs::create_dir(&directory).unwrap();
        for kind in [
            EventKind::Create(CreateKind::Any),
            EventKind::Modify(ModifyKind::Any),
        ] {
            let event = notify::Event {
                kind,
                paths: vec![text.clone()],
                attrs: Default::default(),
            };
            assert_eq!(incremental_paths(&event), Some(vec![text.clone()]));
            let mut pending = Pending::default();
            pending.add(Ok(event), &[root.clone()], &tine_core::Config::default());
            assert_eq!(pending.paths, HashSet::from([text.clone()]));
            assert!(!pending.full);
        }
        // A directory's own timestamps (Windows reports the parent of every
        // written file) need no rescan; a non-text file still does.
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Any),
            paths: vec![directory.clone()],
            attrs: Default::default(),
        };
        assert_eq!(incremental_paths(&event), Some(Vec::new()));
        let event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Any),
            paths: vec![pages.join("image.png")],
            attrs: Default::default(),
        };
        assert_eq!(incremental_paths(&event), None);
        // A directory appearing still needs the full diff.
        let event = notify::Event {
            kind: EventKind::Create(CreateKind::Any),
            paths: vec![directory],
            attrs: Default::default(),
        };
        assert_eq!(incremental_paths(&event), None);
        // Windows reports every removal as `Remove(Any)`; a text path is
        // reread and found missing.
        let removed = notify::Event {
            kind: EventKind::Remove(RemoveKind::Any),
            paths: vec![text.clone()],
            attrs: Default::default(),
        };
        assert_eq!(incremental_paths(&removed), Some(vec![text]));
        fs::remove_dir_all(root).unwrap();
    }

    /// GH #623: the events Tine's own atomic save produces (temp write, then
    /// rename onto the page) name exactly the page; none forces the full stat
    /// diff. Sequences per notify 6.1.1's inotify and ReadDirectoryChangesW
    /// backends; the temp file is gone by the time the callback runs.
    #[test]
    fn an_own_atomic_save_event_sequence_stays_incremental() {
        use notify::event::{
            AccessKind, AccessMode, CreateKind, DataChange, EventKind, ModifyKind, RemoveKind,
            RenameMode,
        };
        let root = temp_root("own-save-events");
        let pages = root.join("pages");
        fs::create_dir_all(&pages).unwrap();
        let page = pages.join("Page.md");
        fs::write(&page, "- saved\n").unwrap();
        let temp = pages.join(".Page.md.4242.7.tmp");
        let ev = |kind, paths: Vec<PathBuf>| notify::Event {
            kind,
            paths,
            attrs: Default::default(),
        };
        let linux = vec![
            ev(EventKind::Create(CreateKind::File), vec![temp.clone()]),
            ev(
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                vec![temp.clone()],
            ),
            ev(
                EventKind::Access(AccessKind::Close(AccessMode::Write)),
                vec![temp.clone()],
            ),
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                vec![temp.clone()],
            ),
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                vec![page.clone()],
            ),
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                vec![temp.clone(), page.clone()],
            ),
        ];
        let windows = vec![
            ev(EventKind::Create(CreateKind::Any), vec![temp.clone()]),
            ev(EventKind::Modify(ModifyKind::Any), vec![temp.clone()]),
            ev(EventKind::Modify(ModifyKind::Any), vec![pages.clone()]),
            ev(EventKind::Remove(RemoveKind::Any), vec![page.clone()]),
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                vec![temp.clone()],
            ),
            ev(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                vec![page.clone()],
            ),
            ev(EventKind::Modify(ModifyKind::Any), vec![page.clone()]),
        ];
        for (platform, events) in [("linux", linux), ("windows", windows)] {
            let mut pending = Pending::default();
            for event in events {
                pending.add(Ok(event), &[root.clone()], &tine_core::Config::default());
            }
            let (paths, full, _) = pending.drain();
            assert!(!full, "{platform}: an own save forced the full stat diff");
            assert_eq!(paths, HashSet::from([page.clone()]), "{platform}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// GH #623, at the real watcher: a page save through the store is followed
    /// by no full stat diff of the graph.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_page_save_never_triggers_a_full_stat_diff() {
        let root = temp_root("own-save-no-full-diff");
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/A.md"), "- old\n").unwrap();
        let store = Store::open(
            &root,
            OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Notify,
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let core = store.watch.core_for_load();
        let before = core.event_full_diffs.load(Ordering::Relaxed);
        let id = crate::PageId::from("pages/A.md");
        for round in 0..3 {
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = format!("edit {round}");
            let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
            tx.save_page(
                &[crate::EditKind::ReplacePage],
                &id,
                crate::SaveBase::Existing(read.rev),
                &doc,
            );
            assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
            std::thread::sleep(Duration::from_millis(600));
        }
        assert_eq!(
            core.event_full_diffs.load(Ordering::Relaxed) - before,
            0,
            "a save was followed by a full stat diff"
        );
        // A real external edit is still seen.
        fs::write(root.join("pages/A.md"), "- external\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !store.page(&id).unwrap().doc.blocks[0]
            .raw
            .contains("external")
        {
            assert!(Instant::now() < deadline, "external edit never observed");
            std::thread::sleep(Duration::from_millis(50));
        }
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_reread_keeps_old_view_and_retries_unchanged_file() {
        let root = std::env::temp_dir().join(format!(
            "tine-watch-reread-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        let path = root.join("pages/A.md");
        fs::write(&path, "- old\n").unwrap();
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
        let writer = store.writer.lock().unwrap();
        let core = store.watch.core_for_load();
        fs::write(&path, "- new content\n").unwrap();
        store
            .graph
            .fail_sync_read_once
            .store(true, Ordering::Release);
        core.reconcile_locked(None, true, true).unwrap();
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| { page.name == "A" && page.document.roots[0].raw().contains("old") }));
        assert!(store
            .graph
            .unreadable_pages()
            .iter()
            .any(|(id, _)| id.as_str() == "pages/A.md"));
        core.reconcile_locked(None, true, true).unwrap();
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| {
                page.name == "A" && page.document.roots[0].raw().contains("new content")
            }));
        assert!(!store
            .graph
            .unreadable_pages()
            .iter()
            .any(|(id, _)| id.as_str() == "pages/A.md"));
        let subscription = store.subscribe();
        fs::write(&path, "- changed again\n").unwrap();
        core.force_mismatched_rev_once
            .store(true, Ordering::Release);
        core.reconcile_locked(None, true, true).unwrap();
        assert!(subscription.try_recv().unwrap().is_none());
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| {
                page.name == "A" && page.document.roots[0].raw().contains("new content")
            }));
        core.reconcile_locked(None, true, true).unwrap();
        let change = subscription
            .try_recv()
            .unwrap()
            .expect("retried page publication");
        assert_eq!(
            change.page(&FileId::from("pages/A.md".to_owned())),
            Some((tine_core::model::PageKind::Page, "A"))
        );
        assert!(store
            .whole_graph()
            .unwrap()
            .corpus()
            .pages
            .iter()
            .any(|page| {
                page.name == "A" && page.document.roots[0].raw().contains("changed again")
            }));
        drop(writer);
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
