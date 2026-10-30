//! Memory-only storage evidence. Each terminal outcome adds one relaxed
//! atomic increment; no names, payloads, file I/O or extra locks enter here.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

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
}

impl StorageCounters {
    pub(crate) fn adopted(&self, reason: DraftAdoptionReason) {
        let counter = match reason {
            DraftAdoptionReason::AppContainerMove => &self.container_adoptions,
            DraftAdoptionReason::UserRecovery => &self.user_adoptions,
        };
        counter.fetch_add(1, Relaxed);
    }

    pub(crate) fn snapshot(&self) -> serde_json::Value {
        let report = serde_json::json!({
            "scope": "since-launch",
            "pageSaves": {
                "published": self.published.load(Relaxed),
                "uncertain": self.uncertain.load(Relaxed),
                "failedOrRefused": self.failed.load(Relaxed),
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
