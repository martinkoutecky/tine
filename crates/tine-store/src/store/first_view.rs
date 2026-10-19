//! Publications before the first complete view (BqbeQ, D-20261019-B6), and
//! the journal-day index they keep current.
//!
//! While the initial load builds the graph and no view exists yet (no
//! launch checkpoint is served), capturing a view would build the whole
//! graph cache on the publishing thread under the writer: O(P + B), 9 s on
//! a 10k-page graph, which an own save during loading used to wait for.
//! Such a publication is deferred instead. Its writes stand and its pages
//! read from their files at once; the cache keeps the rows (the launch
//! pass takes them in, `model/launch_patch.rs`); and the load worker, still
//! holding the writer after the Ready publication, publishes each deferred
//! change in order. So the first complete view contains every deferred
//! write, and each save's own change follows it.
use super::*;
use std::collections::HashSet;

/// A publication held back until the first complete view.
struct DeferredChange {
    origin: Origin,
    files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    pages: Vec<(FileId, PageKind, String)>,
    /// Paths an external writer raced the own write on: reconciled after
    /// this change publishes, as an undeferred publication does at once.
    raced: HashSet<PathBuf>,
}

#[derive(Default)]
pub(super) struct Deferred {
    changes: Vec<DeferredChange>,
    /// A publication was deferred since open, also one with no change of
    /// its own (rows an ownership transition retired).
    any: bool,
}

impl Store {
    /// Whether a publication now waits for the first complete view: the
    /// initial load is still running and no view exists yet. A config
    /// change never waits (the load re-reads with the new config). The
    /// caller holds the writer, which orders this with the Ready
    /// publication.
    pub(super) fn defers_publication(&self, config_changed: bool) -> bool {
        !config_changed && self.first_view_pending()
    }

    /// The initial load is running and no view exists yet: no launch
    /// checkpoint is served, so `whole_graph` would wait for the parse. Early
    /// answers from the files (an unclaimed journal day, a named template)
    /// run only then; with a served checkpoint the view answers at once.
    pub fn first_view_pending(&self) -> bool {
        matches!(*self.load.status.lock().unwrap(), LoadStatus::Loading)
            && self.changes.snapshot.read().unwrap().is_none()
    }

    /// Defer a transaction's publication (see the module doc). A journal
    /// file created or removed updates the day index from the journal
    /// directory's file names (no file opened), so `journal_id` names it
    /// before the first view.
    pub(super) fn defer_publication(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        pages: Vec<(FileId, PageKind, String)>,
        (raced, journal_set_changed): (HashSet<PathBuf>, bool),
    ) {
        if journal_set_changed {
            *self.journal_ids.lock().unwrap() =
                journal_ids_from_entries(&self.graph, &self.graph.scan_journal_names());
        }
        let mut deferred = self.changes.deferred.lock().unwrap();
        deferred.any = true;
        deferred.changes.push(DeferredChange {
            origin,
            files,
            pages,
            raced,
        });
    }

    /// Note that rows an ownership transition retired wait for the first
    /// view's capture, which takes them (`Graph::take_retired`).
    pub(super) fn defer_retired(&self) {
        self.changes.deferred.lock().unwrap().any = true;
    }

    pub(crate) fn refresh_journal_ids(&self) {
        let found = journal_ids_from_entries(&self.graph, self.graph.list_pages_shared().as_ref());
        *self.journal_ids.lock().unwrap() = found;
    }
}

impl ChangeFeed {
    /// Whether a publication was deferred since open: a launch checkpoint,
    /// which lacks its write, must not be served (the watcher's baseline
    /// already holds the write's own stamp, so no diff would correct it).
    pub(super) fn deferred_any(&self) -> bool {
        self.deferred.lock().unwrap().any
    }

    /// The launch pass's files written since open, less those a deferred
    /// publication names: its own change, after the first view, announces
    /// them (an own save is not announced as an outside write).
    pub(super) fn not_deferred(
        &self,
        mut written: Vec<(PathBuf, bool, PageKind, String)>,
    ) -> Vec<(PathBuf, bool, PageKind, String)> {
        let deferred = self.deferred.lock().unwrap();
        let named: HashSet<&str> = (deferred.changes.iter())
            .flat_map(|change| change.files.iter().map(|(id, _, _)| id.as_str()))
            .collect();
        written.retain(|(path, ..)| !named.contains(self.graph.rel_path(path).as_str()));
        written
    }

    /// End deferral. With `watch` (the first view was just published, and
    /// the caller still holds the writer), publish each deferred change in
    /// order; without (the load stopped), drop them: the recovery's first
    /// view covers their writes, as for any publication after a failed load.
    pub(super) fn end_deferral(&self, watch: Option<&crate::watch::Core>) {
        let deferred = std::mem::take(&mut *self.deferred.lock().unwrap());
        let Some(watch) = watch else {
            return;
        };
        for change in deferred.changes {
            // Named from the cache now: a later deferred save of the same
            // page may have renamed it since.
            self.publish_observed(
                change.origin,
                change.files,
                false,
                change.pages,
                &HashMap::new(),
            );
            watch.reconcile_raced(&change.raced);
        }
    }
}

pub(crate) fn journal_ids_from_entries(
    graph: &Graph,
    entries: &[PageEntry],
) -> HashMap<Day, PageId> {
    let mut claimants: HashMap<Day, Vec<PageEntry>> = HashMap::new();
    for entry in entries {
        if entry.kind == PageKind::Journal {
            if let Some(day) = entry.date_key {
                claimants.entry(Day(day)).or_default().push(entry.clone());
            }
        }
    }
    claimants
        .into_iter()
        .filter_map(|(day, mut entries)| {
            entries.sort_by(|a, b| {
                crate::model::compare_page_claimants(
                    a,
                    b,
                    &graph.current_journal_format(),
                    graph.current_config().file_name_format,
                )
            });
            entries.into_iter().next()?.rel_path.map(|id| (day, id))
        })
        .collect()
}
