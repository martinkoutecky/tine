//! Memory-only storage evidence. Each terminal outcome adds one relaxed
//! atomic increment; no names, payloads, file I/O or extra locks enter here.
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering::Relaxed};

// Directory sync has no graph context (it also serves private app-data).
// Count API outcomes process-wide, without retaining paths or adding locks.
#[cfg(target_os = "windows")]
static WINDOWS_DIRECTORY_FLUSH_SUCCEEDED: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "windows")]
static WINDOWS_DIRECTORY_FLUSH_FAILED: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "windows")]
pub(crate) fn record_windows_directory_flush(succeeded: bool) {
    let counter = if succeeded {
        &WINDOWS_DIRECTORY_FLUSH_SUCCEEDED
    } else {
        &WINDOWS_DIRECTORY_FLUSH_FAILED
    };
    counter.fetch_add(1, Relaxed);
}

// Retain counts when a graph closes and reopens during this process. The path
// is only an in-memory lookup key and never enters the exported snapshot.
pub(crate) fn for_graph(root: &std::path::Path) -> std::sync::Arc<StorageCounters> {
    use std::sync::{Arc, Mutex, OnceLock};
    static GRAPHS: OnceLock<
        Mutex<std::collections::BTreeMap<std::path::PathBuf, Arc<StorageCounters>>>,
    > = OnceLock::new();
    GRAPHS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(root.into())
        .or_default()
        .clone()
}

/// Why a device-local draft namespace was adopted. Closed privacy-safe tokens.
#[derive(Clone, Copy)]
pub enum DraftAdoptionReason {
    /// The iOS Documents container changed its absolute location.
    AppContainerMove,
    /// The user explicitly recovered orphan drafts into this graph.
    UserRecovery,
}

/// The post-rename proof a page save could not make, so it ended Uncertain
/// (GH #811: every save of one Android 16 phone was Uncertain and its toast
/// named no step). Closed tokens; no names, paths or stamp values.
#[derive(Clone, Copy)]
pub(crate) enum UncertainProof {
    /// The parent directory's sync failed (its step and errno are in the toast).
    DirectorySync,
    /// The post-rename read established no stamp for the published file.
    PublicationUnread,
    /// No regular file stood at the page path after the directory sync.
    PublicationMissing,
    /// The file at the page path had other metadata after the directory sync
    /// than the post-rename read's handle (M3-1).
    PublicationChanged(StampChange),
}

/// How the post-sync stamp differs from the publication stamp: which fields,
/// and the time and size deltas (actual minus published), never the values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StampChange {
    pub modified_ns: Option<i64>,
    pub size: Option<i64>,
    pub identity_device: bool,
    pub identity_file: bool,
    pub change_time_ns: Option<i64>,
}

/// `i64::MIN` stands for "no delta recorded" (never a real delta here).
const NO_DELTA: i64 = i64::MIN;

#[derive(Default)]
struct UncertainProofs {
    directory_sync: AtomicU64,
    publication_unread: AtomicU64,
    publication_missing: AtomicU64,
    publication_changed: AtomicU64,
    changed_modified: AtomicU64,
    changed_size: AtomicU64,
    changed_identity_device: AtomicU64,
    changed_identity_file: AtomicU64,
    changed_change_time: AtomicU64,
    last_modified_ns: AtomicI64,
    last_size: AtomicI64,
    last_change_time_ns: AtomicI64,
}

impl UncertainProofs {
    fn record(&self, proof: UncertainProof) {
        let counter = match proof {
            UncertainProof::DirectorySync => &self.directory_sync,
            UncertainProof::PublicationUnread => &self.publication_unread,
            UncertainProof::PublicationMissing => &self.publication_missing,
            UncertainProof::PublicationChanged(change) => {
                for (changed, counter) in [
                    (change.modified_ns.is_some(), &self.changed_modified),
                    (change.size.is_some(), &self.changed_size),
                    (change.identity_device, &self.changed_identity_device),
                    (change.identity_file, &self.changed_identity_file),
                    (change.change_time_ns.is_some(), &self.changed_change_time),
                ] {
                    if changed {
                        counter.fetch_add(1, Relaxed);
                    }
                }
                self.last_modified_ns
                    .store(change.modified_ns.unwrap_or(NO_DELTA), Relaxed);
                self.last_size
                    .store(change.size.unwrap_or(NO_DELTA), Relaxed);
                self.last_change_time_ns
                    .store(change.change_time_ns.unwrap_or(NO_DELTA), Relaxed);
                &self.publication_changed
            }
        };
        counter.fetch_add(1, Relaxed);
    }

    fn snapshot(&self) -> serde_json::Value {
        let delta = |value: &AtomicI64| match value.load(Relaxed) {
            NO_DELTA => serde_json::Value::Null,
            value => value.into(),
        };
        let changed = self.publication_changed.load(Relaxed);
        // The deltas hold a value only once a change was recorded.
        let last = (changed > 0).then(|| {
            serde_json::json!({
                "modifiedDeltaNs": delta(&self.last_modified_ns),
                "sizeDelta": delta(&self.last_size),
                "changeTimeDeltaNs": delta(&self.last_change_time_ns),
            })
        });
        serde_json::json!({
            "directorySync": self.directory_sync.load(Relaxed),
            "publicationUnread": self.publication_unread.load(Relaxed),
            "publicationMissing": self.publication_missing.load(Relaxed),
            "publicationChanged": changed,
            "changedFields": {
                "modified": self.changed_modified.load(Relaxed),
                "size": self.changed_size.load(Relaxed),
                "identityDevice": self.changed_identity_device.load(Relaxed),
                "identityFile": self.changed_identity_file.load(Relaxed),
                "changeTime": self.changed_change_time.load(Relaxed),
            },
            "lastChange": last,
        })
    }
}

#[derive(Default)]
pub(crate) struct StorageCounters {
    pub published: AtomicU64,
    pub uncertain: AtomicU64,
    pub failed: AtomicU64,
    pub drafts_written: AtomicU64,
    pub drafts_recovered: AtomicU64,
    /// Case moves a launch finished from their temporary entry (GS1).
    pub case_moves_completed: AtomicU64,
    /// Temporary case-move entries a launch kept in trash: their spelling was taken.
    pub case_moves_kept_in_trash: AtomicU64,
    container_adoptions: AtomicU64,
    user_adoptions: AtomicU64,
    uncertain_proofs: UncertainProofs,
}

impl StorageCounters {
    pub(crate) fn adopted(&self, reason: DraftAdoptionReason) {
        let counter = match reason {
            DraftAdoptionReason::AppContainerMove => &self.container_adoptions,
            DraftAdoptionReason::UserRecovery => &self.user_adoptions,
        };
        counter.fetch_add(1, Relaxed);
    }

    /// Which proof a page save that ended Uncertain could not make (GH #811).
    pub(crate) fn uncertain_proof(&self, proof: UncertainProof) {
        self.uncertain_proofs.record(proof);
    }

    pub(crate) fn snapshot(&self) -> serde_json::Value {
        #[allow(unused_mut)]
        let mut report = serde_json::json!({
            "scope": "since-launch",
            "pageSaves": {
                "published": self.published.load(Relaxed),
                "uncertain": self.uncertain.load(Relaxed),
                "failedOrRefused": self.failed.load(Relaxed),
                // Page saves only; a deletion's Uncertain trash steps are not split.
                "uncertainProof": self.uncertain_proofs.snapshot(),
            },
            "draftsWritten": self.drafts_written.load(Relaxed),
            "draftsRecovered": self.drafts_recovered.load(Relaxed),
            "caseMovesFinishedAtLaunch": {
                "completed": self.case_moves_completed.load(Relaxed),
                "keptInTrash": self.case_moves_kept_in_trash.load(Relaxed),
            },
            "draftNamespaceAdoptions": {
                "appContainerMove": self.container_adoptions.load(Relaxed),
                "userRecovery": self.user_adoptions.load(Relaxed),
            },
            // Tine has no general storage-backend classifier. Do not infer
            // a filesystem or cloud provider from a graph's private path.
            "backendClass": "unclassified",
        });
        #[cfg(target_os = "android")]
        {
            report["androidMtimeReset"] = serde_json::json!({
                "scope": "process-since-launch",
                "succeeded": crate::atomic_file::ANDROID_MTIME_RESET_SUCCEEDED.load(Relaxed),
                "failed": crate::atomic_file::ANDROID_MTIME_RESET_FAILED.load(Relaxed),
            });
        }
        #[cfg(target_os = "windows")]
        let report = {
            let mut report = report;
            report["windowsDirectoryFlush"] = serde_json::json!({
                "scope": "process-since-launch",
                "apiSucceeded": WINDOWS_DIRECTORY_FLUSH_SUCCEEDED.load(Relaxed),
                "apiFailed": WINDOWS_DIRECTORY_FLUSH_FAILED.load(Relaxed),
            });
            report
        };
        report
    }
}
