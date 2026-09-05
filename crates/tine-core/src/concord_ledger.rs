//! Concord base ledger — the per-page "last text Tine agreed on with the disk".
//!
//! For every graph-relative page path this stores the last content Tine
//! successfully READ from disk (external-change admission) or WROTE to disk
//! (a committed save). That text is the true common ancestor of any later
//! "Tine edited + disk changed" divergence, which upgrades the 2-way conflict
//! diff (`crate::sync_diff`) to a real 3-way merge with per-row suggestions.
//! See `docs/adr/0056-concord-base-ledger-and-three-way.md` and spec L2.
//!
//! SEMANTICS — a disposable cache, never an authority:
//! - It lives OUTSIDE the sync tree (`<app_data>/concord-ledger/<root-id>/`),
//!   so transports never see it and it can never pollute a user's graph.
//! - A missing, stale, or corrupt ledger degrades to today's 2-way behavior.
//!   Reads verify the blob's content hash and answer `None` on any mismatch or
//!   I/O error; nothing here may ever block open/save/reload (house rule G2:
//!   caches are disposable; refusing is almost never right).
//! - Writes are enqueued to one background worker thread and are best-effort:
//!   failures are logged to stderr, never surfaced. The foreground cost of an
//!   update is one channel send.
//!
//! Layout (all files atomic tmp+rename, schema `LEDGER_SCHEMA`):
//! - `blobs/<sha256-of-content>`         — content bytes (content-addressed)
//! - `index/<sha256-of-rel-path>.json`   — `{schema, path, hash}`
//! - `pins/<sha256-of-conflict-rel>.json`— `{schema, conflict_path, winner_path, hash}`
//!
//! PINS: when a sync-tool conflict copy is first observed, the winner's
//! then-current ledger entry is pinned under the conflict copy's identity.
//! Without this, admitting the winner's post-sync bytes would overwrite the
//! very base the conflict resolution needs (the admission would make
//! `base == mine` and the 3-way collapse to "take theirs everywhere"). A pin is
//! first-wins — the earliest observation is closest to the true ancestor — and
//! is dropped when its conflict copy is resolved or discarded.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;

/// On-disk schema of index/pin entries; named by `docs/storage-sync-contract.md`
/// §4 (a doc-code consistency test below keeps the two in step). Bumping it
/// invalidates the disposable ledger (entries with another schema read as
/// "no base") and costs nothing but a repopulation.
pub const LEDGER_SCHEMA: u32 = 1;

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Serialize, Deserialize)]
struct IndexEntry {
    schema: u32,
    /// Graph-relative page path (for pruning/debugging; the filename is its hash).
    path: String,
    /// sha256 of the blob's exact bytes.
    hash: String,
}

#[derive(Serialize, Deserialize)]
struct PinEntry {
    schema: u32,
    conflict_path: String,
    winner_path: String,
    hash: String,
}

/// What a prune pass did (for tests and diagnostics).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PruneStats {
    pub blobs_removed: usize,
    pub blobs_kept: usize,
    pub corrupt_entries_removed: usize,
    /// Index/pin entries whose blob is gone (antivirus quarantine, a disk
    /// cleaner, a partial restore). Their lookups already answer `None`
    /// forever, so the entry is dead metadata; prune reclaims it.
    pub dangling_entries_removed: usize,
}

enum Job {
    Record {
        rel: String,
        content: String,
    },
    Pin {
        conflict_rel: String,
        winner_rel: String,
    },
    DropPin {
        conflict_rel: String,
    },
    Prune,
    Flush(mpsc::Sender<()>),
}

/// The per-graph ledger handle. Cheap to construct; performs no I/O until used.
pub struct ConcordLedger {
    dir: PathBuf,
    tx: Mutex<Option<mpsc::Sender<Job>>>,
}

impl ConcordLedger {
    pub fn new(dir: PathBuf) -> Self {
        ConcordLedger {
            dir,
            tx: Mutex::new(None),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    // ---- async surface (the hooks; one channel send on the caller) ---------

    /// Record `content` as the last-agreed text for `rel`, off-thread.
    pub fn record(&self, rel: &str, content: &str) {
        self.enqueue(Job::Record {
            rel: rel.to_string(),
            content: content.to_string(),
        });
    }

    /// Pin the winner's current base under a conflict copy's identity (first-wins).
    pub fn pin_conflict_base(&self, conflict_rel: &str, winner_rel: &str) {
        self.enqueue(Job::Pin {
            conflict_rel: conflict_rel.to_string(),
            winner_rel: winner_rel.to_string(),
        });
    }

    /// Drop a conflict copy's pin (the copy was resolved or discarded).
    pub fn drop_pin(&self, conflict_rel: &str) {
        self.enqueue(Job::DropPin {
            conflict_rel: conflict_rel.to_string(),
        });
    }

    /// Queue a prune of blobs unreferenced by the index (run at graph open).
    pub fn queue_prune(&self) {
        self.enqueue(Job::Prune);
    }

    /// Wait until every previously enqueued job has been processed. Test /
    /// measurement aid; never called on a hot path.
    pub fn flush(&self) {
        let (done_tx, done_rx) = mpsc::channel();
        self.enqueue(Job::Flush(done_tx));
        // 30 s bounds a wedged worker; the ledger is disposable, so give up.
        let _ = done_rx.recv_timeout(std::time::Duration::from_secs(30));
    }

    fn enqueue(&self, job: Job) {
        let mut guard = self.tx.lock().unwrap();
        if guard.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let store = LedgerStore {
                dir: self.dir.clone(),
            };
            std::thread::Builder::new()
                .name("concord-ledger".into())
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        store.run(job);
                    }
                })
                .ok();
            *guard = Some(tx);
        }
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(job);
        }
    }

    // ---- sync surface (reads are on-demand; direct fns also serve tests) ---

    /// The last-agreed text for `rel`, if the ledger holds a verifiable one.
    pub fn base(&self, rel: &str) -> Option<String> {
        self.store().base(rel)
    }

    /// The base to use against a specific conflict copy: its pinned ancestor if
    /// one was captured, else the winner's current entry.
    pub fn conflict_base(&self, conflict_rel: &str, winner_rel: &str) -> Option<String> {
        let store = self.store();
        store
            .pinned_base(conflict_rel)
            .or_else(|| store.base(winner_rel))
    }

    /// Synchronous record — the worker's own path, exposed for tests and
    /// measurement. NOT serialized against the worker thread: `flush()` first
    /// if async jobs (e.g. the attach-time prune) may still be queued.
    pub fn record_now(&self, rel: &str, content: &str) -> io::Result<()> {
        self.store().record(rel, content)
    }

    /// Synchronous pin — exposed for tests.
    pub fn pin_conflict_base_now(&self, conflict_rel: &str, winner_rel: &str) -> io::Result<()> {
        self.store().pin(conflict_rel, winner_rel)
    }

    /// Synchronous prune — exposed for tests.
    pub fn prune_now(&self) -> io::Result<PruneStats> {
        self.store().prune()
    }

    fn store(&self) -> LedgerStore {
        LedgerStore {
            dir: self.dir.clone(),
        }
    }
}

/// The stateless on-disk operations (worker-side; also used for direct reads).
struct LedgerStore {
    dir: PathBuf,
}

impl LedgerStore {
    fn run(&self, job: Job) {
        let outcome = match job {
            Job::Record { rel, content } => self.record(&rel, &content),
            Job::Pin {
                conflict_rel,
                winner_rel,
            } => self.pin(&conflict_rel, &winner_rel),
            Job::DropPin { conflict_rel } => self.drop_pin(&conflict_rel),
            Job::Prune => self.prune().map(|_| ()),
            Job::Flush(done) => {
                let _ = done.send(());
                Ok(())
            }
        };
        if let Err(error) = outcome {
            // Best-effort cache: log, never surface (spec: errors writing the
            // ledger are logged, not surfaced).
            eprintln!("concord-ledger: background update failed: {error}");
        }
    }

    fn blobs_dir(&self) -> PathBuf {
        self.dir.join("blobs")
    }
    fn index_dir(&self) -> PathBuf {
        self.dir.join("index")
    }
    fn pins_dir(&self) -> PathBuf {
        self.dir.join("pins")
    }
    fn index_file(&self, rel: &str) -> PathBuf {
        self.index_dir()
            .join(format!("{}.json", sha256_hex(rel.as_bytes())))
    }
    fn pin_file(&self, conflict_rel: &str) -> PathBuf {
        self.pins_dir()
            .join(format!("{}.json", sha256_hex(conflict_rel.as_bytes())))
    }

    /// Atomic write within the ledger dir (tmp + rename; same filesystem).
    fn write_atomic(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = target.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "ledger path has no parent")
        })?;
        std::fs::create_dir_all(parent)?;
        let tmp = parent.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            sha256_hex(target.as_os_str().as_encoded_bytes())
                .get(..16)
                .unwrap_or("t")
        ));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, target).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    fn record(&self, rel: &str, content: &str) -> io::Result<()> {
        let hash = sha256_hex(content.as_bytes());
        let blob = self.blobs_dir().join(&hash);
        if !blob.exists() {
            self.write_atomic(&blob, content.as_bytes())?;
        }
        let entry = IndexEntry {
            schema: LEDGER_SCHEMA,
            path: rel.to_string(),
            hash,
        };
        let bytes = serde_json::to_vec(&entry).map_err(io::Error::other)?;
        self.write_atomic(&self.index_file(rel), &bytes)
    }

    fn read_index(&self, rel: &str) -> Option<IndexEntry> {
        let bytes = std::fs::read(self.index_file(rel)).ok()?;
        let entry: IndexEntry = serde_json::from_slice(&bytes).ok()?;
        (entry.schema == LEDGER_SCHEMA && entry.path == rel).then_some(entry)
    }

    /// Read + VERIFY a blob by hash; any mismatch means "no base".
    fn read_blob(&self, hash: &str) -> Option<String> {
        let bytes = std::fs::read(self.blobs_dir().join(hash)).ok()?;
        (sha256_hex(&bytes) == hash)
            .then(|| String::from_utf8(bytes).ok())
            .flatten()
    }

    fn base(&self, rel: &str) -> Option<String> {
        self.read_blob(&self.read_index(rel)?.hash)
    }

    fn pin(&self, conflict_rel: &str, winner_rel: &str) -> io::Result<()> {
        let pin_path = self.pin_file(conflict_rel);
        if pin_path.exists() {
            return Ok(()); // first-wins: the earliest pin is closest to the ancestor
        }
        let Some(index) = self.read_index(winner_rel) else {
            return Ok(()); // no base to pin — nothing to do
        };
        let entry = PinEntry {
            schema: LEDGER_SCHEMA,
            conflict_path: conflict_rel.to_string(),
            winner_path: winner_rel.to_string(),
            hash: index.hash,
        };
        let bytes = serde_json::to_vec(&entry).map_err(io::Error::other)?;
        self.write_atomic(&pin_path, &bytes)
    }

    fn pinned_base(&self, conflict_rel: &str) -> Option<String> {
        let bytes = std::fs::read(self.pin_file(conflict_rel)).ok()?;
        let entry: PinEntry = serde_json::from_slice(&bytes).ok()?;
        if entry.schema != LEDGER_SCHEMA || entry.conflict_path != conflict_rel {
            return None;
        }
        self.read_blob(&entry.hash)
    }

    fn drop_pin(&self, conflict_rel: &str) -> io::Result<()> {
        match std::fs::remove_file(self.pin_file(conflict_rel)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Delete blobs referenced by no index entry and no pin, and drop index and
    /// pin files that can no longer answer: unparseable ones, and ones naming a
    /// blob that is absent. `record` writes a blob before the index entry that
    /// names it, so an entry without its blob means the blob was removed from
    /// outside the ledger; the entry is dead and every lookup through it
    /// already answers `None`.
    fn prune(&self) -> io::Result<PruneStats> {
        let mut stats = PruneStats::default();
        let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
        for dir in [self.index_dir(), self.pins_dir()] {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let is_pin_dir = dir.ends_with("pins");
            for entry in entries.flatten() {
                let path = entry.path();
                let hash = std::fs::read(&path).ok().and_then(|bytes| {
                    if is_pin_dir {
                        serde_json::from_slice::<PinEntry>(&bytes)
                            .ok()
                            .filter(|p| p.schema == LEDGER_SCHEMA)
                            .map(|p| p.hash)
                    } else {
                        serde_json::from_slice::<IndexEntry>(&bytes)
                            .ok()
                            .filter(|i| i.schema == LEDGER_SCHEMA)
                            .map(|i| i.hash)
                    }
                });
                match hash {
                    Some(hash) if self.blobs_dir().join(&hash).exists() => {
                        referenced.insert(hash);
                    }
                    Some(_) => {
                        stats.dangling_entries_removed += 1;
                        let _ = std::fs::remove_file(&path);
                    }
                    None => {
                        stats.corrupt_entries_removed += 1;
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
        let blobs = match std::fs::read_dir(self.blobs_dir()) {
            Ok(blobs) => blobs,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(stats),
            Err(error) => return Err(error),
        };
        for entry in blobs.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with(".tmp-") || !referenced.contains(name) {
                stats.blobs_removed += 1;
                let _ = std::fs::remove_file(entry.path());
            } else {
                stats.blobs_kept += 1;
            }
        }
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tine-concord-ledger-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Doc-code consistency (house rule: living contracts carry tested values):
    /// the storage contract's §4 must keep describing the ledger as disposable,
    /// non-authoritative, Direct-Files-only, and at the current schema.
    #[test]
    fn storage_contract_names_the_concord_ledger_and_its_schema() {
        let contract = include_str!("../../../docs/storage-sync-contract.md");
        assert!(contract.contains("## 4. Concord base ledger (Direct Files)"));
        assert!(contract.contains("concord-ledger/<root-id>"));
        assert!(contract.contains(&format!("currently {LEDGER_SCHEMA}")));
        assert!(contract.contains("It is never an authority"));
        assert!(contract.contains("safe to delete wholesale at any time"));
        assert!(contract.contains("a managed\nbinding never attaches one"));
        assert!(contract.contains("index and pin entries naming a blob that is absent"));
        assert!(contract.contains("never warns,\nrefuses, or reports a missing blob to the user"));
    }

    #[test]
    fn round_trips_the_last_agreed_text_per_path() {
        let ledger = ConcordLedger::new(scratch("roundtrip"));
        assert_eq!(ledger.base("pages/A.md"), None);
        ledger.record_now("pages/A.md", "- one\n").unwrap();
        ledger.record_now("pages/B.md", "- two\n").unwrap();
        assert_eq!(ledger.base("pages/A.md").as_deref(), Some("- one\n"));
        assert_eq!(ledger.base("pages/B.md").as_deref(), Some("- two\n"));
        // A later record replaces the entry (last agreed wins).
        ledger.record_now("pages/A.md", "- one edited\n").unwrap();
        assert_eq!(ledger.base("pages/A.md").as_deref(), Some("- one edited\n"));
        std::fs::remove_dir_all(ledger.dir()).ok();
    }

    #[test]
    fn async_record_lands_after_flush() {
        let ledger = ConcordLedger::new(scratch("async"));
        ledger.record("pages/A.md", "- queued\n");
        ledger.flush();
        assert_eq!(ledger.base("pages/A.md").as_deref(), Some("- queued\n"));
        std::fs::remove_dir_all(ledger.dir()).ok();
    }

    #[test]
    fn corrupt_blob_or_index_degrades_to_no_base_without_error() {
        let ledger = ConcordLedger::new(scratch("corrupt"));
        ledger.record_now("pages/A.md", "- good content\n").unwrap();
        let hash = sha256_hex(b"- good content\n");
        // Corrupt the blob bytes: the hash check must reject it.
        std::fs::write(ledger.dir().join("blobs").join(&hash), b"tampered").unwrap();
        assert_eq!(ledger.base("pages/A.md"), None);
        // Corrupt the index file: parse failure must degrade, not panic.
        ledger.record_now("pages/C.md", "- c\n").unwrap();
        let index = ledger.dir().join("index");
        for entry in std::fs::read_dir(&index).unwrap().flatten() {
            std::fs::write(entry.path(), b"{not json").unwrap();
        }
        assert_eq!(ledger.base("pages/C.md"), None);
        std::fs::remove_dir_all(ledger.dir()).ok();
    }

    #[test]
    fn prune_removes_unreferenced_blobs_and_keeps_pinned_ones() {
        let ledger = ConcordLedger::new(scratch("prune"));
        ledger.record_now("pages/A.md", "- version 1\n").unwrap();
        // Pin version 1 under a conflict identity, then move A on to version 2.
        ledger
            .pin_conflict_base_now("pages/A.sync-conflict-x.md", "pages/A.md")
            .unwrap();
        ledger.record_now("pages/A.md", "- version 2\n").unwrap();
        // An orphan blob nothing references.
        std::fs::write(
            ledger.dir().join("blobs").join(sha256_hex(b"orphan")),
            b"orphan",
        )
        .unwrap();
        let stats = ledger.prune_now().unwrap();
        assert_eq!(stats.blobs_removed, 1, "{stats:?}");
        assert_eq!(stats.blobs_kept, 2, "{stats:?}"); // v1 (pinned) + v2 (indexed)
        assert_eq!(ledger.base("pages/A.md").as_deref(), Some("- version 2\n"));
        assert_eq!(
            ledger
                .conflict_base("pages/A.sync-conflict-x.md", "pages/A.md")
                .as_deref(),
            Some("- version 1\n"),
            "the pinned ancestor must survive the newer record and the prune"
        );
        std::fs::remove_dir_all(ledger.dir()).ok();
    }

    #[test]
    fn prune_reclaims_index_and_pin_entries_whose_blob_vanished() {
        // GH #411 residue: antivirus quarantine (or a disk cleaner, or a partial
        // restore) removes blobs from under the ledger. Lookups already degrade
        // to `None`, but the entries naming those blobs used to survive every
        // prune as dead metadata.
        let ledger = ConcordLedger::new(scratch("prune-dangling"));
        ledger.record_now("pages/A.md", "- version 1\n").unwrap();
        ledger
            .pin_conflict_base_now("pages/A.sync-conflict-x.md", "pages/A.md")
            .unwrap();
        ledger.record_now("pages/B.md", "- b\n").unwrap();

        // Remove every blob, exactly as an external cleaner would.
        for entry in std::fs::read_dir(ledger.dir().join("blobs"))
            .unwrap()
            .flatten()
        {
            std::fs::remove_file(entry.path()).unwrap();
        }
        assert_eq!(ledger.base("pages/A.md"), None, "lookup already degrades");

        let index_dir = ledger.dir().join("index");
        let pins_dir = ledger.dir().join("pins");
        let count = |dir: &std::path::Path| std::fs::read_dir(dir).unwrap().count();
        assert_eq!(count(&index_dir), 2, "two index entries before the prune");
        assert_eq!(count(&pins_dir), 1, "one pin before the prune");

        let stats = ledger.prune_now().unwrap();
        assert_eq!(stats.dangling_entries_removed, 3, "{stats:?}");
        assert_eq!(stats.corrupt_entries_removed, 0, "{stats:?}");
        assert_eq!(count(&index_dir), 0, "dangling index entries are reclaimed");
        assert_eq!(count(&pins_dir), 0, "dangling pins are reclaimed");

        // Recording again after the cleanup still works and still answers.
        ledger.record_now("pages/A.md", "- version 2\n").unwrap();
        assert_eq!(ledger.base("pages/A.md").as_deref(), Some("- version 2\n"));
        let stats = ledger.prune_now().unwrap();
        assert_eq!(stats.dangling_entries_removed, 0, "{stats:?}");
        assert_eq!(stats.blobs_kept, 1, "{stats:?}");
        std::fs::remove_dir_all(ledger.dir()).ok();
    }

    #[test]
    fn pin_is_first_wins_and_dropped_on_request() {
        let ledger = ConcordLedger::new(scratch("pin"));
        ledger.record_now("pages/A.md", "- ancestor\n").unwrap();
        ledger
            .pin_conflict_base_now("pages/A.sync-conflict-y.md", "pages/A.md")
            .unwrap();
        ledger.record_now("pages/A.md", "- newer\n").unwrap();
        // A second pin attempt must NOT overwrite the earlier (closer) ancestor.
        ledger
            .pin_conflict_base_now("pages/A.sync-conflict-y.md", "pages/A.md")
            .unwrap();
        assert_eq!(
            ledger
                .conflict_base("pages/A.sync-conflict-y.md", "pages/A.md")
                .as_deref(),
            Some("- ancestor\n")
        );
        // With no pin, conflict_base falls back to the winner's current entry.
        assert_eq!(
            ledger
                .conflict_base("pages/A.sync-conflict-z.md", "pages/A.md")
                .as_deref(),
            Some("- newer\n")
        );
        ledger.drop_pin("pages/A.sync-conflict-y.md");
        ledger.flush();
        assert_eq!(
            ledger
                .conflict_base("pages/A.sync-conflict-y.md", "pages/A.md")
                .as_deref(),
            Some("- newer\n"),
            "after the pin is dropped the fallback is the current entry"
        );
        std::fs::remove_dir_all(ledger.dir()).ok();
    }
}
