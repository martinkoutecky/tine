//! External asset observation (master d017d1afc / 2f54a8d5e; GH #38, GH #127).
//!
//! Assets are ordinary files that an editor, Syncthing, Dropbox or another
//! Tine window may replace under a rendered image. This lane sees only file
//! metadata: it never reads asset bytes, never hashes, never enters the page
//! cache, and publishes `Origin::External` changes whose `assets/<rel>` ids
//! carry no revision. Its scope is the graph's canonical assets capability
//! (`Graph::assets_path`): `<root>/assets`, or the exact approved external
//! target of an `assets` link. Own writes update the baseline through
//! [`AssetObserver::note_own`], so they never echo back as external.
//!
//! Unit cost: at most one extra OS watch (only for an approved external assets
//! root outside the graph; an in-graph assets directory rides the graph-root
//! watch), one `HashMap` entry per asset file, one stat per event path. A full
//! scan (poll cycle, kernel rescan, explicit `refresh`) costs O(asset
//! files) stats and zero reads.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::model::Graph;
use crate::store::ChangeKind;
use crate::watch::{stamp_from_metadata, stamp_metadata, Stamp};

/// Where the asset capability lives, and the lexical spelling under the graph
/// root that a recursive watch may report for it (notify follows links).
#[derive(Clone, Debug)]
pub(crate) struct AssetScope {
    root: PathBuf,
    lexical: PathBuf,
}

impl AssetScope {
    pub(crate) fn new(graph: &Graph) -> Self {
        Self {
            root: graph.assets_path(),
            lexical: graph.root.join("assets"),
        }
    }

    /// The approved assets directory when it lies outside the graph root and
    /// therefore needs a watch of its own.
    pub(crate) fn external_root(&self, graph_root: &Path) -> Option<&Path> {
        (!self.root.starts_with(graph_root)).then_some(self.root.as_path())
    }

    /// `path` spelled under the assets capability (either spelling), mapped to
    /// the canonical root. Never canonicalizes: a deleted file must still map.
    fn owned(&self, path: &Path) -> Option<PathBuf> {
        let relative = path
            .strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&self.lexical))
            .ok()?;
        Some(self.root.join(relative))
    }

    /// Like [`Self::owned`], but never the assets directory itself.
    fn strictly_inside(&self, path: &Path) -> Option<PathBuf> {
        let mapped = self.owned(path)?;
        let relative = mapped.strip_prefix(&self.root).ok()?;
        (!relative.as_os_str().is_empty()).then_some(mapped)
    }

    fn touches(&self, path: &Path, ancestor_matters: bool) -> bool {
        self.owned(path).is_some()
            || (ancestor_matters && (self.root.starts_with(path) || self.lexical.starts_with(path)))
    }
}

/// Whether the file at `path` (under the assets directory `root`) is one of
/// Tine's own writes: a temp of any of its atomic publishers
/// ([`crate::atomic_file::is_temp_name`], the one temp-name answer), or a file
/// in restore recovery ([`in_restore_recovery`]). Only the final rename is a
/// logical asset change. The temp rule is for files only: a folder with a
/// temp-shaped name is the user's and is walked (model check of c897398d8).
fn is_tine_file(path: &Path, root: &Path) -> bool {
    let atomic_temp = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(crate::atomic_file::is_temp_name);
    atomic_temp || in_restore_recovery(path.strip_prefix(root).unwrap_or(path))
}

/// Restore's recovery folder under the assets directory, the one `.tine-`
/// entry Tine keeps there: restore makes it, and the asset watcher and
/// restore's own walks leave it out ([`in_restore_recovery`]).
pub(crate) const ASSET_RECOVERY: &str = ".tine-restore-recovery";

/// The one answer to "is this asset path inside restore recovery": whether a
/// component of `relative` (a path relative to the assets directory) is
/// restore's recovery folder ([`ASSET_RECOVERY`]). Its callers are the asset
/// watcher, asset admission during restore, restore's walk of the assets
/// directory and the restore baseline (`watch/restore.rs`). A separate
/// question from "is this a temp name": the folder is no temp, and every file
/// inside it is Tine's whatever its name. Exactly that name: a user's
/// `.tine-report.tmp` is the user's (review of 6fb074e9f). Costs one compare
/// per component; no I/O.
pub(crate) fn in_restore_recovery(relative: &Path) -> bool {
    relative
        .components()
        .any(|part| part.as_os_str() == ASSET_RECOVERY)
}

/// Events queued between watcher cycles: exact ordinary-file paths and whether
/// a full metadata scan is due.
#[derive(Default)]
pub(crate) struct AssetPending {
    exact: HashSet<PathBuf>,
    full: bool,
}

impl AssetPending {
    /// Queue what the event means for the asset lane. True when every path of
    /// the event lies strictly inside the assets directory: such an event can
    /// never describe graph text, so the page lane skips it entirely (an asset
    /// write used to cost a full graph-text stat diff).
    pub(crate) fn note(
        &mut self,
        event: &notify::Result<notify::Event>,
        scope: &AssetScope,
    ) -> bool {
        use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};
        let Ok(event) = event else {
            self.full = true;
            return false;
        };
        if event.need_rescan() {
            self.full |=
                event.paths.is_empty() || event.paths.iter().any(|path| scope.touches(path, true));
            return false;
        }
        // A metadata/data event on an ancestor directory says nothing about
        // the files below it; a create/remove/rename of one does.
        let ancestor_matters = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
        );
        if !event
            .paths
            .iter()
            .any(|path| scope.touches(path, ancestor_matters))
        {
            return false;
        }
        let exact_kind = matches!(
            event.kind,
            EventKind::Create(CreateKind::File | CreateKind::Any)
                | EventKind::Modify(
                    ModifyKind::Data(_)
                        | ModifyKind::Metadata(_)
                        | ModifyKind::Any
                        | ModifyKind::Name(RenameMode::From | RenameMode::To | RenameMode::Both)
                )
                | EventKind::Remove(RemoveKind::File)
        );
        if exact_kind && !event.paths.iter().any(|path| path.is_dir()) {
            self.exact.extend(
                event
                    .paths
                    .iter()
                    .filter_map(|path| scope.strictly_inside(path)),
            );
        } else {
            self.full = true;
        }
        event
            .paths
            .iter()
            .all(|path| scope.strictly_inside(path).is_some())
    }

    /// Registration cannot report changes made since the opening baseline.
    /// Keep this obligation queued even while graph loading prevents a cycle.
    pub(crate) fn scan_after_install(&mut self) {
        self.full = true;
    }

    pub(crate) fn drain(&mut self) -> (HashSet<PathBuf>, bool) {
        (
            std::mem::take(&mut self.exact),
            std::mem::take(&mut self.full),
        )
    }
}

fn collect(root: &Path) -> (HashMap<PathBuf, Stamp>, bool) {
    let mut files = HashMap::new();
    let mut complete = true;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == root && error.kind() == std::io::ErrorKind::NotFound => {
                continue
            }
            Err(_) => {
                complete = false;
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                complete = false;
                continue;
            };
            // No-follow: a link inside the directory cannot widen the capability.
            if kind.is_dir() {
                // Only restore recovery prunes a subtree; a temp-shaped
                // folder name is no temp.
                if !in_restore_recovery(path.strip_prefix(root).unwrap_or(&path)) {
                    stack.push(path);
                }
            } else if kind.is_file() {
                if is_tine_file(&path, root) {
                    continue;
                }
                #[cfg(feature = "test-faults")]
                crate::cost_counters::asset_listing_stamp();
                // The listing's own metadata (no-follow; on Windows from
                // FindNextFileW), so a walk opens no file: a per-file
                // `symlink_metadata` here cost one open per asset, under the
                // store's writer lock, on every focus rescan (GH #623).
                match entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| stamp_from_metadata(&metadata))
                {
                    Some(stamp) => {
                        files.insert(path, stamp);
                    }
                    None => complete = false,
                }
            }
        }
    }
    (files, complete)
}

/// The per-graph asset baseline. Captured before the OS watch is installed so
/// a replacement in the handoff gap differs from it instead of becoming it.
pub(crate) struct AssetObserver {
    scope: AssetScope,
    snapshot: Mutex<HashMap<PathBuf, Stamp>>,
}

impl AssetObserver {
    pub(crate) fn new(scope: AssetScope) -> Self {
        let snapshot = collect(&scope.root).0;
        Self {
            scope,
            snapshot: Mutex::new(snapshot),
        }
    }

    pub(crate) fn scope(&self) -> &AssetScope {
        &self.scope
    }

    /// One of this store's own writes is now the baseline: no external echo.
    pub(crate) fn note_own(&self, path: &Path) {
        let Some(path) = self.scope.strictly_inside(path) else {
            return;
        };
        let mut snapshot = self.snapshot.lock().unwrap();
        match stamp_metadata(&path) {
            Some(stamp) => snapshot.insert(path, stamp),
            None => snapshot.remove(&path),
        };
    }

    /// Compare disk with the baseline for the queued paths (or everything when
    /// `full`), advance the baseline, and say what changed. Metadata only.
    pub(crate) fn reconcile(
        &self,
        exact: &HashSet<PathBuf>,
        full: bool,
    ) -> Vec<(PathBuf, ChangeKind)> {
        let mut snapshot = self.snapshot.lock().unwrap();
        let mut changed = Vec::new();
        if full {
            let (current, complete) = collect(&self.scope.root);
            for (path, stamp) in &current {
                match snapshot.get(path) {
                    None => changed.push((path.clone(), ChangeKind::Created)),
                    Some(old) if old != stamp => changed.push((path.clone(), ChangeKind::Modified)),
                    Some(_) => {}
                }
            }
            if complete {
                // A partial walk proves present changes, never deletion.
                changed.extend(
                    snapshot
                        .keys()
                        .filter(|path| !current.contains_key(*path))
                        .map(|path| (path.clone(), ChangeKind::Removed)),
                );
                *snapshot = current;
            } else {
                snapshot.extend(current);
            }
        } else {
            for path in exact
                .iter()
                .filter(|path| !is_tine_file(path, &self.scope.root))
            {
                let now = stamp_metadata(path);
                match (snapshot.get(path), now) {
                    (None, None) => {}
                    (None, Some(stamp)) => {
                        snapshot.insert(path.clone(), stamp);
                        changed.push((path.clone(), ChangeKind::Created));
                    }
                    (Some(_), None) => {
                        snapshot.remove(path);
                        changed.push((path.clone(), ChangeKind::Removed));
                    }
                    (Some(old), Some(stamp)) if *old != stamp => {
                        snapshot.insert(path.clone(), stamp);
                        changed.push((path.clone(), ChangeKind::Modified));
                    }
                    (Some(_), Some(_)) => {}
                }
            }
        }
        changed.sort_by(|a, b| a.0.cmp(&b.0));
        changed.dedup_by(|a, b| a.0 == b.0);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review of 6fb074e9f: both asset-watcher paths (an exact event and a
    /// full rescan) skip every name Tine writes under the assets directory and
    /// nothing else: a user's file shaped like a temp, inside a `.tine-`
    /// folder of the user's own, or inside a folder named like a temp, is an
    /// asset change.
    #[test]
    fn the_asset_watcher_skips_exactly_the_names_tine_writes() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let assets = root.join("assets");
        fs::create_dir_all(assets.join(ASSET_RECOVERY).join("restore-1")).unwrap();
        fs::create_dir_all(assets.join(".tine-mine")).unwrap();
        // A folder with a temp-shaped name is the user's folder: the temp rule
        // is for files (model check of c897398d8).
        fs::create_dir_all(assets.join(".a.png.4242.21.capture.tmp")).unwrap();
        let graph = Graph::open(&root);
        let exact_observer = AssetObserver::new(AssetScope::new(&graph));
        let full_observer = AssetObserver::new(AssetScope::new(&graph));

        let mut tine: Vec<PathBuf> = crate::atomic_file::tine_temp_names("pic.png")
            .iter()
            .map(|name| assets.join(name))
            .collect();
        tine.push(assets.join(ASSET_RECOVERY).join("restore-1/pic.png"));
        let mut user: Vec<PathBuf> = crate::atomic_file::TEMP_LOOK_ALIKES
            .iter()
            .map(|name| assets.join(name))
            .collect();
        user.push(assets.join(".tine-mine/pic.png"));
        user.push(assets.join(".a.png.4242.21.capture.tmp/photo.png"));
        user.push(assets.join("pic.png"));
        for path in tine.iter().chain(&user) {
            fs::write(path, b"x").unwrap();
        }
        let mut expected: Vec<_> = user
            .iter()
            .map(|path| (path.clone(), ChangeKind::Created))
            .collect();
        expected.sort_by(|a, b| a.0.cmp(&b.0));

        let exact: HashSet<PathBuf> = tine.iter().chain(&user).cloned().collect();
        assert_eq!(exact_observer.reconcile(&exact, false), expected);
        assert_eq!(full_observer.reconcile(&HashSet::new(), true), expected);
    }
}
