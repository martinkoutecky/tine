//! The watcher's launch baseline (GH #623; storage spec §5.1, §7.6). The
//! load pass stamps every graph file before its one read; the baseline is
//! installed from those observations, and one full stat diff before Ready
//! reconciles whatever changed since. The open-time baseline walk and the
//! second hash pass over every file (`fill_revs`) run only as the fallback
//! when an on-demand build filled the cache instead of the load pass.
//!
//! Unit cost: no file is read here except one whose stamp changed or was
//! racy; one stat per graph file (the launch diff); nothing is written.

use std::path::PathBuf;
use std::time::Instant;

use super::{collect_with_errors, Core, Stamp};
use crate::launch_diag::DiffTrigger;
use crate::store::{ChangeKind, FileId, FileRev, LoadError};

/// Where a watcher's graph-text baseline comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Baseline {
    /// A stat walk when the watcher starts (a store built ready, in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    Walk,
    /// The load pass's own observations, installed before the launch diff.
    FromLoad,
}

/// What the launch diff found, published with the Ready publication.
#[derive(Default)]
pub(crate) struct Deferred {
    pub(crate) files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    pub(crate) config_changed: bool,
    pub(crate) pages: Vec<(FileId, tine_core::model::PageKind, String)>,
}

impl Core {
    /// Install the baseline the load pass observed (storage spec §5.1 step 1:
    /// every stamp was taken before its file's read, so any later change
    /// differs from it). An own write noted while loading is newer, and kept.
    /// Caller holds the writer.
    pub(crate) fn install_launch_baseline(&self, observed: crate::model::LaunchObservations) {
        let mut snapshot = self.snapshot.lock().unwrap();
        let mut racy = self.racy.lock().unwrap();
        for (path, value) in observed.stamps {
            if let std::collections::hash_map::Entry::Vacant(slot) = snapshot.entry(path) {
                if observed.racy.contains(slot.key()) {
                    racy.insert(slot.key().clone());
                }
                slot.insert(value);
            }
        }
    }

    /// The baseline when the load pass's observations are not available (an
    /// on-demand build won the race to fill the cache): a stat walk, then a
    /// hash of every file whose stamp still matches. Caller holds the writer.
    pub(crate) fn install_walked_baseline(&self) {
        let began = Instant::now();
        let (walked, _, times) =
            collect_with_errors(&self.dirs.read().unwrap(), &self.graph.current_config());
        self.graph.diag.baseline(times, began.elapsed());
        {
            let mut snapshot = self.snapshot.lock().unwrap();
            for (path, value) in walked {
                snapshot.entry(path).or_insert(value);
            }
        }
        self.fill_revs();
    }

    /// The launch diff (§5.1 step 2): one full stat diff against the
    /// installed baseline, rereading racy paths, before the graph is Ready.
    /// Its changes are returned for the Ready publication. Caller holds the
    /// writer.
    pub(crate) fn launch_diff(&self) -> Result<Deferred, LoadError> {
        let mut deferred = Deferred::default();
        self.reconcile_timed(
            None,
            true,
            false,
            None,
            DiffTrigger::Launch,
            Some(&mut deferred),
        )?;
        Ok(deferred)
    }

    /// Files the load pass found written since open: a client may have read
    /// their earlier bytes while the graph was loading, so the Ready
    /// publication names them (as the open-time baseline walk this replaced
    /// did). A file the launch diff already reports is not repeated.
    pub(crate) fn announce_written_since_open(
        &self,
        written: Vec<(PathBuf, bool, tine_core::model::PageKind, String)>,
        deferred: &mut Deferred,
    ) {
        let snapshot = self.snapshot.lock().unwrap();
        for (path, created, kind, name) in written {
            let Some(id) = self.file_id(&path) else {
                continue;
            };
            if deferred.files.iter().any(|(known, _, _)| *known == id) {
                continue;
            }
            let Some(rev) = snapshot.get(&path).and_then(|value| value.rev.clone()) else {
                continue;
            };
            let change = if created {
                ChangeKind::Created
            } else {
                ChangeKind::Modified
            };
            deferred.files.push((id.clone(), change, Some(rev)));
            deferred.pages.push((id, kind, name));
        }
    }

    /// The baseline a launch checkpoint stores (ADR 0070): every graph-text
    /// stamp, sorted, and the racy set. Caller holds the writer, so no
    /// reconcile moves them while they are cloned.
    pub(crate) fn checkpoint_observations(&self) -> (Vec<(PathBuf, Stamp)>, Vec<PathBuf>) {
        let mut stamps: Vec<(PathBuf, Stamp)> = self
            .snapshot
            .lock()
            .unwrap()
            .iter()
            .map(|(path, value)| (path.clone(), value.clone()))
            .collect();
        stamps.sort_by(|a, b| a.0.cmp(&b.0));
        let racy: Vec<PathBuf> = self.racy.lock().unwrap().iter().cloned().collect();
        (stamps, racy)
    }
}

impl Stamp {
    /// Revision of the bytes read after this observation, if they were read.
    pub(crate) fn rev(&self) -> Option<&FileRev> {
        self.rev.as_ref()
    }
}
