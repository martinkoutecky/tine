//! Concord base ledger (og ADR 0056): per page, the last `RETAINED` distinct
//! texts Tine agreed on with the disk — bytes it saved, or external bytes the
//! store admitted. A sync-conflict review uses one of them as the common
//! ancestor, which upgrades the 2-way diff to a 3-way one whose rows carry
//! suggestions the user applies through the guarded resolve.
//!
//! Questions and operations (all O(one page), never a graph walk except prune):
//! - [`ConcordLedger::observe`]: hand one published store `Change` to the
//!   ledger's worker thread. Caller cost: one channel send. The worker reads
//!   each changed page once (O(page bytes)), records it, and pins the winner's
//!   ancestor when a sync-conflict copy appears.
//! - [`ConcordLedger::conflict_bases`]: candidate ancestors for one copy,
//!   newest first: its pin, then the winner's retained texts. Every blob is
//!   re-hashed on read; anything unreadable, corrupt or foreign answers
//!   nothing, and the caller falls back to the 2-way diff.
//! - [`ConcordLedger::prune`]: at graph open, drop entries for pages and
//!   copies that are gone, blobs no entry names, corrupt entries and torn
//!   temps. O(ledger files).
//! - [`ConcordLedger::drain_for_exit`]: quitting waits at most
//!   [`EXIT_DRAIN_BUDGET`] for queued updates.
//!
//! Never an authority: nothing refuses, delays or fails on the ledger. It is
//! off the save path (fed from the store's change feed after commit), lives in
//! app data outside every graph root (`<app_data>/`[`LEDGER_DIR`]`/<root-id>/`),
//! and every write is `device_io::atomic_write` (temp + fsync + rename +
//! directory sync). Worker errors are logged and dropped. A caller must not
//! need to know the layout, the worker, or retention.
//!
//! Layout (schema [`LEDGER_SCHEMA`]):
//! - `pages/<sha256(rel)>/index.json` — `{schema, path, revs}`, `revs` the
//!   sha256 of the retained texts, newest first, at most [`RETAINED`];
//! - `pages/<sha256(rel)>/<sha256(text)>` — one retained text;
//! - `pins/<sha256(copy rel)>.json` — `{schema, conflict_path, winner_path, hash}`;
//! - `pins/<sha256(copy rel)>.blob` — the pinned text (its own copy, so
//!   retention never evicts it). Dropped when the copy leaves the graph.

use crate::state::GraphSlot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex, Weak};
use std::time::{Duration, Instant};
use tine_store::{Change, ChangeKind, FileId};

/// Schema of index and pin entries; another schema reads as "no base".
pub(crate) const LEDGER_SCHEMA: u32 = 1;
/// Distinct texts kept per page (Martin, og QUESTIONS Q4: K=2). Two survive a
/// watcher admission that makes the newest entry equal to the winner.
pub(crate) const RETAINED: usize = 2;
/// How long quitting waits in total for queued ledger updates (master ADR
/// 0056, Martin 2026-09-15): the save made just before quitting is the base
/// the next device's conflict needs; the bound keeps a wedged disk from
/// holding up the exit.
pub(crate) const EXIT_DRAIN_BUDGET: Duration = Duration::from_millis(200);
/// og's own app-data folder. Master Tine keeps an incompatible ledger layout
/// under `concord-ledger/`; once og and master share one app-data directory
/// (the planned identity flip, or a rollback) each build's prune would delete
/// the other's entries. A separate folder means og never reads, prunes or
/// writes master's tree (and the reverse). Disposable, so no migration.
pub(crate) const LEDGER_DIR: &str = "concord-ledger-og";

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Serialize, Deserialize)]
struct PageIndex {
    schema: u32,
    path: String,
    revs: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Pin {
    schema: u32,
    conflict_path: String,
    winner_path: String,
    hash: String,
}

enum Job {
    Observe(Change),
    Prune,
    Flush(mpsc::Sender<()>),
}

/// One graph window's ledger handle. Performs no I/O until used.
pub(crate) struct ConcordLedger {
    dir: PathBuf,
    slot: Weak<GraphSlot>,
    tx: Mutex<Option<mpsc::Sender<Job>>>,
}

impl ConcordLedger {
    pub(crate) fn new(dir: PathBuf, slot: Weak<GraphSlot>) -> Self {
        Self {
            dir,
            slot,
            tx: Mutex::new(None),
        }
    }

    /// Record the published change off-thread (see the module doc).
    pub(crate) fn observe(&self, change: &Change) {
        if !change.files.is_empty() {
            self.enqueue(Job::Observe(change.clone()));
        }
    }

    /// Queue the open-time prune.
    pub(crate) fn queue_prune(&self) {
        self.enqueue(Job::Prune);
    }

    /// Candidate ancestors for `copy` (graph-relative), newest first: the pin,
    /// then `winner`'s retained texts. Verified; empty on any failure.
    pub(crate) fn conflict_bases(&self, copy: &str, winner: &str) -> Vec<String> {
        let files = self.files();
        let mut out: Vec<String> = files.pinned(copy).into_iter().collect();
        for text in files.retained(winner) {
            if !out.contains(&text) {
                out.push(text);
            }
        }
        out
    }

    /// Every retained text of `page` (graph-relative), newest first. Verified;
    /// empty on any failure. A live-draft review picks its base among these.
    pub(crate) fn page_bases(&self, page: &str) -> Vec<String> {
        self.files().retained(page)
    }

    /// Wait until every job queued so far ran, or `deadline` passed; answers
    /// whether they all ran. Never starts the worker.
    pub(crate) fn drain_for_exit(&self, deadline: Instant) -> bool {
        let (done, wait) = mpsc::channel();
        {
            let guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
            let Some(tx) = guard.as_ref() else {
                return true;
            };
            if tx.send(Job::Flush(done)).is_err() {
                return false;
            }
        }
        wait.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok()
    }

    fn files(&self) -> LedgerFiles {
        LedgerFiles {
            dir: self.dir.clone(),
        }
    }

    fn enqueue(&self, job: Job) {
        let mut guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let files = self.files();
            let slot = self.slot.clone();
            let spawned = std::thread::Builder::new()
                .name("concord-ledger".into())
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        let outcome = match job {
                            Job::Observe(change) => match slot.upgrade() {
                                Some(slot) => observe(&files, &slot.store, &change),
                                None => Ok(()),
                            },
                            Job::Prune => match slot.upgrade() {
                                Some(slot) => files.prune(&slot.store).map(|_| ()),
                                None => Ok(()),
                            },
                            Job::Flush(done) => {
                                let _ = done.send(());
                                Ok(())
                            }
                        };
                        if let Err(error) = outcome {
                            crate::debug::diag_private(
                                "concord-ledger-update-failed",
                                format!("concord-ledger: {error}"),
                            );
                        }
                    }
                });
            if spawned.is_err() {
                return;
            }
            *guard = Some(tx);
        }
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(job);
        }
    }
}

/// The worker's handling of one change: pins for newly observed copies first
/// (so a winner admission in the same change cannot become the pinned base),
/// dropped pins for copies that left, then one record per changed page whose
/// bytes still match the published revision (a later change records newer
/// bytes). O(changed page bytes).
fn observe(files: &LedgerFiles, store: &tine_store::Store, change: &Change) -> io::Result<()> {
    use tine_graph_features::conflicts::sync_copy_of;
    let read = |rel: &str| {
        store
            .read(
                &FileId::from(rel.to_owned()),
                Some(tine_store::PARSE_INPUT_MAX_BYTES),
            )
            .ok()
    };
    for (id, kind, _) in &change.files {
        let Some(winner) = sync_copy_of(store, id) else {
            continue;
        };
        match kind {
            ChangeKind::Created => {
                let now = |rel: &str| read(rel).map(|(bytes, _)| sha(&bytes));
                files.pin(id.as_str(), &winner, [now(&winner), now(id.as_str())])?
            }
            ChangeKind::Removed => files.drop_pin(id.as_str())?,
            _ => {}
        }
    }
    for (id, kind, rev) in &change.files {
        let (Some(rev), ChangeKind::Created | ChangeKind::Modified) = (rev, kind) else {
            continue;
        };
        if sync_copy_of(store, id).is_some() || store.as_page(id).is_none() {
            continue;
        }
        if let Some((bytes, read_rev)) = read(id.as_str()) {
            if &read_rev == rev {
                files.record(id.as_str(), &bytes)?;
            }
        }
    }
    Ok(())
}

/// Stateless on-disk operations over one ledger directory.
struct LedgerFiles {
    dir: PathBuf,
}

impl LedgerFiles {
    fn page_dir(&self, rel: &str) -> PathBuf {
        self.dir.join("pages").join(sha(rel.as_bytes()))
    }

    fn pin_path(&self, copy: &str, ext: &str) -> PathBuf {
        self.dir
            .join("pins")
            .join(format!("{}.{ext}", sha(copy.as_bytes())))
    }

    fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::device_io::atomic_write(path, bytes)
    }

    fn read_verified(path: &Path, hash: &str) -> Option<String> {
        let bytes = std::fs::read(path).ok()?;
        (sha(&bytes) == hash)
            .then(|| String::from_utf8(bytes).ok())
            .flatten()
    }

    fn index(&self, rel: &str) -> Option<PageIndex> {
        let bytes = std::fs::read(self.page_dir(rel).join("index.json")).ok()?;
        let index: PageIndex = serde_json::from_slice(&bytes).ok()?;
        (index.schema == LEDGER_SCHEMA && index.path == rel).then_some(index)
    }

    /// Make `bytes` the newest retained text of `rel`: one blob (when new) and
    /// one index rewrite, then the blob that fell out of retention is removed.
    /// Re-recording the newest text writes nothing.
    fn record(&self, rel: &str, bytes: &[u8]) -> io::Result<()> {
        let hash = sha(bytes);
        let old = self.index(rel).map(|i| i.revs).unwrap_or_default();
        if old.first() == Some(&hash) {
            return Ok(());
        }
        let dir = self.page_dir(rel);
        if !old.contains(&hash) {
            Self::write(&dir.join(&hash), bytes)?;
        }
        let mut revs: Vec<String> = old.iter().filter(|h| **h != hash).cloned().collect();
        revs.insert(0, hash);
        let evicted = revs.split_off(revs.len().min(RETAINED));
        let index = PageIndex {
            schema: LEDGER_SCHEMA,
            path: rel.to_owned(),
            revs,
        };
        Self::write(
            &dir.join("index.json"),
            &serde_json::to_vec(&index).map_err(io::Error::other)?,
        )?;
        for hash in evicted {
            let _ = std::fs::remove_file(dir.join(hash));
        }
        Ok(())
    }

    fn retained(&self, rel: &str) -> Vec<String> {
        let dir = self.page_dir(rel);
        self.index(rel)
            .map(|index| {
                index
                    .revs
                    .iter()
                    .filter_map(|hash| Self::read_verified(&dir.join(hash), hash))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// First pin wins: the earliest observation is closest to the ancestor.
    /// Pins the newest retained text of `winner` that differs from both files'
    /// current bytes (`now`): a text equal to either side is an admission
    /// artifact, not their ancestor.
    fn pin(&self, copy: &str, winner: &str, now: [Option<String>; 2]) -> io::Result<()> {
        let json = self.pin_path(copy, "json");
        if json.exists() {
            return Ok(());
        }
        let dir = self.page_dir(winner);
        let Some(index) = self.index(winner) else {
            return Ok(());
        };
        let Some((hash, text)) = index
            .revs
            .iter()
            .filter(|hash| !now.iter().flatten().any(|n| n == *hash))
            .find_map(|hash| Some((hash.clone(), Self::read_verified(&dir.join(hash), hash)?)))
        else {
            return Ok(());
        };
        Self::write(&self.pin_path(copy, "blob"), text.as_bytes())?;
        let pin = Pin {
            schema: LEDGER_SCHEMA,
            conflict_path: copy.to_owned(),
            winner_path: winner.to_owned(),
            hash,
        };
        Self::write(&json, &serde_json::to_vec(&pin).map_err(io::Error::other)?)
    }

    fn pin_entry(&self, copy: &str) -> Option<Pin> {
        let bytes = std::fs::read(self.pin_path(copy, "json")).ok()?;
        let pin: Pin = serde_json::from_slice(&bytes).ok()?;
        (pin.schema == LEDGER_SCHEMA && pin.conflict_path == copy).then_some(pin)
    }

    fn pinned(&self, copy: &str) -> Option<String> {
        let pin = self.pin_entry(copy)?;
        Self::read_verified(&self.pin_path(copy, "blob"), &pin.hash)
    }

    fn drop_pin(&self, copy: &str) -> io::Result<()> {
        for ext in ["json", "blob"] {
            match std::fs::remove_file(self.pin_path(copy, ext)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Ok(())
    }

    /// Drop what can no longer answer: page entries whose page is gone or
    /// whose index is unreadable, texts no index names (torn temps included),
    /// index revisions whose text is missing, and pins whose copy is gone or
    /// whose text is missing. Returns the number of files removed.
    fn prune(&self, store: &tine_store::Store) -> io::Result<usize> {
        let exists = |rel: &str| store.open_read(&FileId::from(rel.to_owned())).is_ok();
        let mut removed = 0;
        let list = |dir: PathBuf| -> Vec<PathBuf> {
            std::fs::read_dir(dir)
                .map(|it| it.flatten().map(|e| e.path()).collect())
                .unwrap_or_default()
        };
        for page in list(self.dir.join("pages")) {
            let index = std::fs::read(page.join("index.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<PageIndex>(&b).ok())
                .filter(|i| i.schema == LEDGER_SCHEMA && self.page_dir(&i.path) == page);
            let Some(mut index) = index.filter(|i| exists(&i.path)) else {
                removed += list(page.clone()).len();
                std::fs::remove_dir_all(&page)?;
                continue;
            };
            let before = index.revs.len();
            index.revs.retain(|hash| page.join(hash).is_file());
            if index.revs.len() != before {
                Self::write(
                    &page.join("index.json"),
                    &serde_json::to_vec(&index).map_err(io::Error::other)?,
                )?;
            }
            for file in list(page.clone()) {
                let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name != "index.json" && !index.revs.iter().any(|h| h == name) {
                    std::fs::remove_file(&file)?;
                    removed += 1;
                }
            }
        }
        let mut pins = list(self.dir.join("pins"));
        // Entries before blobs, so a blob whose entry is dropped goes too.
        pins.sort_by_key(|file| file.extension().is_some_and(|ext| ext == "blob"));
        for file in pins {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let keep = match name.strip_suffix(".json") {
                Some(stem) => std::fs::read(&file)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Pin>(&b).ok())
                    .is_some_and(|pin| {
                        pin.schema == LEDGER_SCHEMA
                            && sha(pin.conflict_path.as_bytes()) == stem
                            && exists(&pin.conflict_path)
                            && Self::read_verified(&file.with_extension("blob"), &pin.hash)
                                .is_some()
                    }),
                None => name
                    .strip_suffix(".blob")
                    .is_some_and(|stem| file.with_file_name(format!("{stem}.json")).is_file()),
            };
            if !keep {
                std::fs::remove_file(&file)?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// Attach a ledger to a freshly bound graph slot and queue its prune. A
/// missing app-data directory leaves the slot without a ledger (2-way diffs).
pub(crate) fn attach(app_data: Option<PathBuf>, slot: &std::sync::Arc<GraphSlot>) {
    let Some(app_data) = app_data else {
        return;
    };
    let dir = app_data
        .join(LEDGER_DIR)
        .join(crate::backup::root_backup_id(&slot.root_key));
    let ledger = ConcordLedger::new(dir, std::sync::Arc::downgrade(slot));
    ledger.queue_prune();
    let _ = slot.concord_ledger.set(ledger);
}

/// Quitting: give every bound graph's queued ledger updates one shared
/// [`EXIT_DRAIN_BUDGET`].
pub(crate) fn drain_all_for_exit(state: &crate::state::AppState) {
    let deadline = Instant::now() + EXIT_DRAIN_BUDGET;
    let slots = state
        .graphs
        .read()
        .map(|graphs| graphs.entries())
        .unwrap_or_default();
    for (_, slot) in slots {
        if let Some(ledger) = slot.concord_ledger.get() {
            ledger.drain_for_exit(deadline);
        }
    }
}

#[cfg(test)]
#[path = "concord_ledger_tests.rs"]
mod tests;
