//! Memory-only storage evidence. Each terminal outcome adds one relaxed
//! atomic increment; no names, payloads, file I/O or extra locks enter here.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

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
        serde_json::json!({
            "scope": "since-launch",
            "pageSaves": {
                "published": self.published.load(Relaxed),
                "uncertain": self.uncertain.load(Relaxed),
                "failedOrRefused": self.failed.load(Relaxed),
            },
            "draftsWritten": self.drafts_written.load(Relaxed),
            "draftsRecovered": self.drafts_recovered.load(Relaxed),
            "draftNamespaceAdoptions": {
                "appContainerMove": self.container_adoptions.load(Relaxed),
                "userRecovery": self.user_adoptions.load(Relaxed),
            },
            // Tine has no general storage-backend classifier. Do not infer
            // a filesystem or cloud provider from a graph's private path.
            "backendClass": "unclassified",
        })
    }
}
