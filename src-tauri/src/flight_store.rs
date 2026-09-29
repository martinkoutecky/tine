//! Persisted half of the diagnostic flight recorder (og ADR 0058; Martin's Q5
//! approval, 2026-09-29).
//!
//! **Questions answered.** [`FlightStore::open`] — did the previous run end
//! without an orderly shutdown, and which of its events survived?
//! **Operations accepted.** [`FlightStore::write_history`] republishes this
//! run's events; [`FlightStore::set_session_active`] arms or clears the
//! unclean-exit marker; [`FlightStore::save_report`] publishes a report the user
//! chose to save.
//!
//! **Layout.** One directory, `<app data>/diagnostics/`, never inside a graph:
//! [`HISTORY_FILE`] (this run's fixed-shape events, one JSON object per line,
//! at most [`crate::flight::FLIGHT_MAX_BYTES`]), [`MARKER_FILE`] (empty; present
//! while a session is live) and [`LOCK_FILE`] (empty; held by the one process
//! that owns the directory). Every write is `device_io::atomic_write` (temp +
//! fsync + rename + directory sync).
//!
//! **Refusals.** None refuses startup. A lock another live process holds makes
//! this process record in memory only (scenario: honest concurrent Tine
//! instance — a forwarded second launch must not rotate the primary's
//! evidence). An unreadable, oversized or malformed history is discarded line
//! by line and rebuilt by the next write (scenario: crash/power loss or disk
//! error leaving a partial file; the history is a disposable cache).
//!
//! **Cost.** Open reads at most [`HISTORY_READ_CAP`] bytes once per launch.
//! A history write is O(retained bytes) ≤ 1 MiB, one file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use serde_json::Value;

pub(crate) const HISTORY_FILE: &str = "history.jsonl";
pub(crate) const MARKER_FILE: &str = "session-active";
pub(crate) const LOCK_FILE: &str = "process.lock";
/// A history larger than the recorder can ever write is not one of ours.
pub(crate) const HISTORY_READ_CAP: u64 = crate::flight::FLIGHT_MAX_BYTES as u64;

pub(crate) struct FlightStore {
    dir: PathBuf,
    lock: File,
}

/// What a launch learns about the run before it.
pub(crate) struct Opened {
    pub(crate) store: FlightStore,
    pub(crate) previous_unclean: bool,
    /// The previous run's valid event lines, oldest first.
    pub(crate) previous: Vec<String>,
}

/// `flock`/`LockFileEx` belong to the open file description; unlock explicitly
/// so a descriptor a spawned child inherited cannot keep the directory owned.
impl Drop for FlightStore {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
    }
}

impl FlightStore {
    /// Take the directory, read the previous run, and arm the marker for this
    /// one. `WouldBlock` means another live Tine owns the directory.
    pub(crate) fn open(dir: &Path) -> io::Result<Opened> {
        fs::create_dir_all(dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK_FILE))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(io::ErrorKind::WouldBlock.into()),
            // A platform without advisory locks still gets its history: the
            // single-instance plugin already forwards a second desktop launch.
            Err(fs::TryLockError::Error(_)) => {}
        }
        let store = Self {
            dir: dir.to_path_buf(),
            lock,
        };
        let previous_unclean = fs::symlink_metadata(store.dir.join(MARKER_FILE)).is_ok();
        let previous = read_history(&store.dir.join(HISTORY_FILE));
        store.set_session_active(true)?;
        Ok(Opened {
            store,
            previous_unclean,
            previous,
        })
    }

    /// Replace the history with `encoded` (newline-terminated event lines).
    pub(crate) fn write_history(&self, encoded: &[u8]) -> io::Result<()> {
        crate::device_io::atomic_write(&self.dir.join(HISTORY_FILE), encoded)
    }

    /// Present marker = "a death from here on is unexpected". Cleared at an
    /// orderly exit, and on mobile when the OS may reap the hidden app.
    pub(crate) fn set_session_active(&self, active: bool) -> io::Result<()> {
        let marker = self.dir.join(MARKER_FILE);
        if active {
            return crate::device_io::atomic_write(&marker, b"");
        }
        match fs::remove_file(marker) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Publish a report at a destination the user chose in the save dialog.
    pub(crate) fn save_report(destination: &Path, text: &str) -> io::Result<()> {
        crate::device_io::atomic_write(destination, text.as_bytes())
    }
}

/// The valid event lines of a history file: each must be a JSON object of
/// this schema with an `event` name. Anything else — a torn last line, bytes a
/// disk error mangled, an oversized file — is dropped. Never fails.
pub(crate) fn read_history(path: &Path) -> Vec<String> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file
        .take(HISTORY_READ_CAP + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > HISTORY_READ_CAP
    {
        return Vec::new();
    }
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|value| {
                value["schemaVersion"] == crate::flight::FLIGHT_SCHEMA_VERSION
                    && value["event"].is_string()
            })
        })
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str) -> String {
        format!("{{\"schemaVersion\":1,\"elapsedMs\":0,\"event\":\"{name}\"}}")
    }

    #[test]
    fn a_run_that_never_cleared_its_marker_is_reported_unclean_once() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        {
            let opened = FlightStore::open(&dir).unwrap();
            assert!(!opened.previous_unclean);
            assert!(dir.join(MARKER_FILE).is_file());
            // dropped without set_session_active(false): a killed process
        }
        let opened = FlightStore::open(&dir).unwrap();
        assert!(opened.previous_unclean);
        opened.store.set_session_active(false).unwrap();
        assert!(!dir.join(MARKER_FILE).exists());
        drop(opened);
        assert!(!FlightStore::open(&dir).unwrap().previous_unclean);
    }

    /// GH #426: a mobile session the OS reaps while hidden is not unclean, but
    /// one that dies after the user came back still is.
    #[test]
    fn a_backgrounded_session_is_clean_until_it_becomes_active_again() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        FlightStore::open(&dir)
            .unwrap()
            .store
            .set_session_active(false)
            .unwrap();
        assert!(!FlightStore::open(&dir).unwrap().previous_unclean);
        let opened = FlightStore::open(&dir).unwrap();
        opened.store.set_session_active(false).unwrap();
        opened.store.set_session_active(true).unwrap();
        drop(opened);
        assert!(FlightStore::open(&dir).unwrap().previous_unclean);
    }

    #[test]
    fn a_truncated_or_corrupt_history_keeps_only_whole_events_and_is_rebuilt() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        fs::create_dir_all(&dir).unwrap();
        let history = dir.join(HISTORY_FILE);
        let torn = format!(
            "{}\nnot json\n{{\"schemaVersion\":2,\"event\":\"future\"}}\n\u{0}\u{ff}\n{}\n{{\"schemaVersion\":1,\"ev",
            event("runtime.started"),
            event("watcher.batch")
        );
        fs::write(&history, torn).unwrap();
        let opened = FlightStore::open(&dir).unwrap();
        assert_eq!(
            opened.previous,
            vec![event("runtime.started"), event("watcher.batch")]
        );
        let rebuilt = format!("{}\n", event("runtime.started"));
        opened.store.write_history(rebuilt.as_bytes()).unwrap();
        assert_eq!(fs::read_to_string(&history).unwrap(), rebuilt);

        fs::write(&history, vec![b'x'; HISTORY_READ_CAP as usize + 1]).unwrap();
        drop(opened);
        assert!(FlightStore::open(&dir).unwrap().previous.is_empty());
    }

    #[test]
    fn a_second_live_owner_is_refused_and_a_dropped_one_releases_the_directory() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("diagnostics");
        let first = FlightStore::open(&dir).unwrap();
        let duplicate = first.store.lock.try_clone().unwrap();
        assert_eq!(
            FlightStore::open(&dir).err().map(|error| error.kind()),
            Some(io::ErrorKind::WouldBlock)
        );
        drop(first);
        // A duplicated descriptor (a child mid-exec) must not keep the lock.
        assert!(FlightStore::open(&dir).is_ok());
        drop(duplicate);
    }

    #[test]
    fn a_saved_report_replaces_its_destination_whole() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("report.json");
        fs::write(&destination, "old and longer contents").unwrap();
        FlightStore::save_report(&destination, "{}").unwrap();
        assert_eq!(fs::read_to_string(destination).unwrap(), "{}");
    }
}
