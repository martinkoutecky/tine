//! Phase seam shared by ModelFs and the production adapter.
use super::Text;

/// A physical step of the host's I/O; tests outside the crate name one to
/// plant a fault there (`super::faults`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// A page file's read.
    Read,
    /// A page's synced temporary file.
    PageTemp,
    /// A page temporary's publication (no-replace create or replace).
    PageRename,
    /// The page directory's sync after a publication.
    PageSync,
    /// A deleted page's move into the transaction trash.
    TrashMove,
    /// The trash move's file and directory syncs.
    TrashSync,
    /// A deletion's custody marker write.
    CustodyWrite,
    /// A custody marker's removal.
    CustodyRetire,
    /// The custody directory's listing.
    CustodyList,
    /// A draft vehicle's temporary file.
    DraftTemp,
    /// A draft vehicle's publication.
    DraftRename,
    /// A draft vehicle's removal.
    DraftUnlink,
    /// The draft directory's sync.
    DraftSync,
    /// An unreadable draft vehicle's quarantine.
    Quarantine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Witness {
    Durable,
    Unsupported,
}

/// A paired source's durability witness (`HostIo::source_witnessed`): the
/// one producer of source evidence, mapped to D/W/U by
/// `Host::source_original`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceWitness {
    /// One handle read, flushed and reread the file unchanged, every
    /// directory from its own up to the graph root synced, and the path
    /// still names that file afterwards.
    Durable,
    /// As `Durable`, but a directory sync reported `Unsupported` (NFS,
    /// FUSE, Android shared storage), or the chain is a Windows one, where a
    /// graph directory sync is no flush: the save path's weak witness.
    Weak,
    /// A step failed: custody is kept.
    Failed(WitnessStep),
}

/// The witness step that failed (one `storage_diag` record each).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WitnessStep {
    /// The file or a directory of its chain failed the graph containment
    /// check.
    Containment,
    /// The file could not be opened (Windows: for writing, which its flush
    /// needs: a read-only attribute, an ACL or a sharing denial).
    Open,
    /// The first read or its metadata call failed.
    Read,
    /// The file's own flush failed.
    FileSync,
    /// The reread or its metadata call failed.
    Reread,
    /// The bytes were not the expected ones, or the file changed between
    /// the read and the reread.
    Changed,
    /// A directory sync of the chain failed.
    DirectorySync,
    /// After the chain sync the path no longer names the same unchanged
    /// file.
    Replaced,
}

impl WitnessStep {
    /// The step's `storage_diag` counter (`SOURCE_WITNESS_STEPS` names it).
    pub(super) fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ErrorKind {
    Io,
    Collision,
}

/// `completed` retains a physical step that succeeded before an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IoFailure {
    pub kind: ErrorKind,
    pub completed: bool,
    /// The platform step that failed, a fixed literal (GH #538, I-5), and
    /// its OS error code: a failed save's notice carries them (Q-P2b-3).
    pub operation: Option<&'static str>,
    pub os_error: Option<i32>,
}

pub(super) type IoResult<T> = Result<T, IoFailure>;

/// A move can remove bytes even when a later phase reports an error.
pub(super) struct MoveResult {
    pub removed: Text,
    pub result: IoResult<()>,
}

/// Crash-recovery availability (plan v3 §4, B-Q1), for the `load_graph`
/// reply and Retry. `unavailable` is why draft I/O is down: while it is,
/// every draft effect fails without touching the filesystem. `unreadable`
/// names vehicles launch could not quarantine; they stay in place untouched.
/// `unsaved` is the persistent indicator at an open (SPEC-s2 §4.11, S9):
/// the pages the host holds whose text is not on disk yet. `kept` counts the
/// originals a cancelled rename kept (`Order::kept`), which the window's
/// recovery panel lists and restores.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftStatus {
    pub(crate) unavailable: Option<String>,
    pub(crate) unreadable: Vec<String>,
    pub(crate) unsaved: Vec<UnsavedPage>,
    pub(crate) kept: usize,
}

/// One original a cancelled rename kept (`PageHost::kept_originals`): the
/// row the recovery panel shows, addressed by (`key`, `origin`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptOriginal {
    /// The source page's host key (opaque to the window).
    pub(crate) key: String,
    /// The source record's write sequence (`Pair::origin`).
    pub(crate) origin: u64,
    /// The source's file, relative to the graph root.
    pub(crate) path: String,
    /// The page the restored file would be, as its creating save names it;
    /// None when the graph's settings no longer make the file a page.
    pub(crate) name: Option<String>,
    /// Whether it would be a page or a journal day.
    pub(crate) kind: tine_core::model::PageKind,
    /// The kept text's length in bytes.
    pub(crate) len: usize,
}

/// The check a restore's locked step could not make (`RestoreRefusal`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RestoreStep {
    /// Reading the page's slot.
    SlotRead,
    /// Looking for another spelling of the page.
    TwinProbe,
}

/// Why a kept original was not restored (`PageHost::restore_kept`). Each one
/// changes nothing: the original stays kept.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "reason", rename_all = "camelCase")]
pub enum RestoreRefusal {
    /// It is no longer kept (restored, released, or another original now).
    NotKept,
    /// The graph's settings no longer make its file a page.
    Ineligible,
    /// A window holds the page, or the page its text names.
    Held {
        /// The page's name.
        name: String,
    },
    /// The page has unsaved input (a recovered draft, admitted input).
    Unsaved,
    /// A file is at its slot.
    Occupied,
    /// Another file is already this page.
    Twin {
        /// That file, relative to the graph root.
        file: String,
    },
    /// A check could not be made.
    Failed {
        /// Which check.
        step: RestoreStep,
        /// The file operation that failed, when known.
        operation: Option<&'static str>,
        /// The OS error code, when there is one.
        #[serde(rename = "osError")]
        os_error: Option<i32>,
    },
    /// Another window session, or a host that is down or stopping.
    Unavailable,
}

/// What became of a restore (`PageHost::restore_kept`), as develop's host
/// operation waiter answers an admitted operation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RestoreOutcome {
    /// The host's own save published the original, and the index has it.
    Restored,
    /// Refused before anything was written.
    Refused(RestoreRefusal),
    /// Other content reached the page before that publication (a Use Disk
    /// took the file's bytes): the original stays kept while its pair
    /// holds it.
    Superseded,
    /// The page cannot be saved yet (a conflict, or saves that keep
    /// failing). `held`: a window holds the page now, and its editor says
    /// so.
    Unwritten {
        /// A window holds the page now.
        held: bool,
    },
    /// Not known within the wait.
    Uncertain,
}

/// A page whose text is not on disk yet, and why (STEP3 §9, B-QA).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsavedPage {
    /// Its file, relative to the graph root.
    pub(crate) path: String,
    /// Launch recovered it from a crash-recovery copy, and nothing changed
    /// it since: Tine is saving it.
    pub(crate) recovered: bool,
    /// Its file changed on disk; the user chooses.
    pub(crate) conflict: bool,
    /// Its saves keep failing (the save-error notice).
    pub(crate) failing: bool,
}

/// One invocation exposes one publication phase. No filesystem access escapes
/// this seam. The 2b adapter must extend the existing audited primitives.
pub(super) trait HostIo {
    /// The key's current spelling (STEP3 §2): page I/O and the key's path
    /// lock use it. A key is its own spelling until `spell` says otherwise.
    fn spelling(&self, key: &str) -> String;
    /// Registration, or the alias spelling move (Q4), names the directory
    /// entry `spelling`. Adapter memory only: a new binding re-registers.
    fn spell(&mut self, key: &str, spelling: &str);
    /// Registered keys in this spelling's folded-leaf bucket, by lookup.
    /// The abstract model has no spelling collisions; production shares B1's table.
    fn folded_keys(&self, _key: &str) -> Vec<String> {
        Vec::new()
    }
    /// Best effort only; it is not a publication or promise. Launch's one
    /// directory pass also witnesses the decided `sources` (page, the bytes
    /// launch read, each its pair's preimage): every source's file steps
    /// first, one handle at a time, then one sync of each directory of
    /// `pages` and `sources` up to the graph root, then each source's final
    /// metadata check. The answer per source is `source_witnessed`'s.
    fn graph_launch(
        &mut self,
        _pages: &std::collections::BTreeSet<String>,
        sources: &[(String, Text)],
    ) -> Vec<SourceWitness> {
        (sources.iter())
            .map(|(page, bytes)| self.source_witnessed(page, bytes))
            .collect()
    }
    /// Release only this save's unpublished temporary vehicle at completion.
    fn page_finish(&mut self, _page: &str) {}
    fn read_page(&mut self, page: &str) -> IoResult<Text>;
    /// The page file's metadata as its last read or own write's publication
    /// read took it from the read's own handle, before the bytes, with
    /// their revision (GH #623): the publication's pre-read stamp, which
    /// spares the watcher another open and hash. Taken once.
    fn take_stamp(&mut self, _page: &str) -> Option<crate::watch::Stamp> {
        None
    }
    fn page_temp(&mut self, page: &str, bytes: &Text) -> IoResult<()>;
    fn page_rename(&mut self, page: &str) -> IoResult<()>;
    fn page_sync(&mut self, page: &str) -> IoResult<Witness>;
    /// A durability witness for the page's current file (a paired
    /// receiver's check, which compares its bytes): it is still the file
    /// this host's own save published and directory-synced, unchanged
    /// since. Another program's bytes carry no witness, and a launch has
    /// none. Adapter memory only. Production has no weak-graph exemption:
    /// on a graph that reports no durability it still answers only from
    /// its own publication, which is stricter than the model's weak
    /// profile (the model certifies on bytes there), never weaker; only
    /// the conformance ledger implements the model's exemption.
    fn durable_page(&mut self, page: &str) -> bool;
    /// A durability witness for a paired source's current file, expected
    /// to hold `bytes` (`Host::source_original` asks only when those are its
    /// pair's preimage): one handle reads the file, takes its metadata,
    /// flushes it, rereads it and takes its metadata again (equal bytes and
    /// stamps); then every directory from the file's own up to the graph
    /// root is synced, root first; then the path must still name that
    /// unchanged file. Asked anew at every custody decision, so a failed
    /// step is never forgotten and an own save that synced only the page's
    /// own directory pays no ancestor's debt. Every failure gets one
    /// `storage_diag` record naming its step.
    fn source_witnessed(&mut self, page: &str, bytes: &Text) -> SourceWitness;
    /// The host no longer needs `page`'s durability witness (it is no
    /// receiver of an unresolved pair): drop it, so the adapter's witnesses
    /// stay bounded by the unresolved pairs and are not one per page ever
    /// saved. A later own publication takes a new one.
    fn release_durable(&mut self, _page: &str) {}
    /// The page file's alternate-extension twin (`.md` ↔ `.org`), if one
    /// exists: another file claiming the page's name (STEP3 §3.2, Q9).
    fn page_twin(&mut self, page: &str) -> IoResult<Option<String>>;
    /// Extra production creation refusal: a published logical claimant or a
    /// possible unreadable owner. Ordinary replacements never call this.
    fn page_owner(
        &mut self,
        _page: &str,
        _bytes: &Text,
        _source: Option<&str>,
    ) -> IoResult<Option<String>> {
        Ok(None)
    }
    /// No-replace move of the page's file to the graph trash as `payload`.
    fn trash_move(&mut self, page: &str, payload: &str) -> MoveResult;
    /// Trash custody (A4): (a) check the payload still exists, then (b) sync
    /// the source directory, the trash directory and each existing ancestor up
    /// to the graph root, so the payload's *name* is durable. The payload's
    /// data is not synced: Tine's own bytes were synced before they got a
    /// name, and another program's unflushed bytes are R-UNFLUSHED (RO3 (a)).
    /// A payload that no longer exists owes nothing (R-PURGE).
    fn trash_sync(&mut self, page: &str, payload: &str) -> IoResult<Witness>;
    /// Strict new-file write of a custody marker into app data.
    fn custody_write(&mut self, name: &str, bytes: &[u8]) -> IoResult<()>;
    /// Unlink a custody marker and sync its app-data directory.
    fn custody_retire(&mut self, name: &str) -> IoResult<()>;
    /// This device's markers, at launch and on progress's retry while the
    /// listing is unknown; never a listing of the trash. Unpublished marker
    /// temps are unlinked first (no move followed them). The error names the
    /// filesystem's failure (REVIEW-2b-r2 V1).
    fn custody_markers(&mut self) -> Result<Vec<(String, Vec<u8>)>, String>;
    /// The readable (`durable == false`) or durable census, in full. Launch
    /// and tests only: a live host reads its draft index instead (D-10).
    fn draft_files(&self, durable: bool) -> Vec<(String, Vec<u8>)>;
    /// Durable census entries changed since the previous call, in order:
    /// the new bytes, or None once removed. A live host keeps its decoded
    /// draft index in step with these instead of rescanning every vehicle.
    fn draft_changes(&mut self) -> Vec<(String, Option<Vec<u8>>)>;
    fn draft_temp(&mut self, name: &str, bytes: &[u8]) -> IoResult<()>;
    fn draft_rename(&mut self, name: &str) -> IoResult<()>;
    fn draft_unlink(&mut self, name: &str) -> IoResult<()>;
    fn draft_sync(&mut self) -> IoResult<Witness>;
    fn quarantine(&mut self, name: &str) -> IoResult<()>;
    fn draft_status(&self) -> DraftStatus;
    /// Launch could not make the recovered census durable (M2): draft I/O
    /// stays down, so no vehicle is retired, until a re-probe succeeds.
    fn drafts_unsynced(&mut self);
    /// Retry (S3): re-probe down draft I/O in place; Ok once it is up.
    fn drafts_reprobe(&mut self) -> Result<(), String>;
}
