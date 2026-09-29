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
//! scan (poll cycle, kernel rescan, explicit `scan_refresh`) costs O(asset
//! files) stats and zero reads.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::model::Graph;
use crate::store::ChangeKind;
use crate::watch::{stamp_metadata, Stamp};

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

/// Tine's atomic publishers leave hidden numeric temp names
/// (`.{name}.{pid}.{seq}[.tag].tmp`), and restore/transactions leave
/// `.tine-*` litter. Only the final rename is a logical asset change.
fn is_tine_bookkeeping(path: &Path, root: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let hidden_tine = relative.components().any(|part| {
        part.as_os_str()
            .to_str()
            .is_some_and(|name| name.starts_with(".tine-"))
    });
    let atomic_temp = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".tmp"))
        .is_some_and(|stem| {
            let mut parts = stem.rsplit('.');
            let numeric = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
            let first = parts.next().unwrap_or_default();
            let seq = if numeric(first) {
                first
            } else {
                parts.next().unwrap_or_default()
            };
            stem.starts_with('.') && numeric(seq) && parts.next().is_some_and(numeric)
        });
    hidden_tine || atomic_temp
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
            if is_tine_bookkeeping(&path, root) {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                match stamp_metadata(&path) {
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
                .filter(|path| !is_tine_bookkeeping(path, &self.scope.root))
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
