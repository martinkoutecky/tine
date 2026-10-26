//! The one stamp producer per platform and the racy-stamp rule (storage spec
//! §1 stamp, §5.4).
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::{FileRev, Stamp};

/// Storage spec §5.4: a stamp whose mtime lies within this window of the
/// moment it was observed is racy. A same-size write landing in the same
/// timestamp granule (or delivered by a sync client that preserves mtimes)
/// can leave the stamp unchanged, so the bytes are reread regardless.
pub(crate) const RACY_WINDOW: Duration = Duration::from_secs(2);

impl Stamp {
    /// The file's modification time as observed.
    pub(crate) fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    /// The file's length as observed.
    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    /// This observation with the revision of the bytes read after it.
    pub(crate) fn with_rev(mut self, rev: Option<FileRev>) -> Self {
        self.rev = rev;
        self
    }

    /// Whether `other` observed the same metadata (time, size, identity and
    /// change time), whatever the revisions. The watcher's own unchanged
    /// test; a racy stamp is still reread on the next poll (§5.4).
    pub(crate) fn same_metadata(&self, other: &Stamp) -> bool {
        self.modified == other.modified
            && self.len == other.len
            && self.identity == other.identity
            && self.changed == other.changed
    }

    /// §5.4: racy when observed at `observed` (judged at observation time;
    /// a missing mtime is racy because nothing bounds it).
    pub(crate) fn racy_at(&self, observed: SystemTime) -> bool {
        match self.modified {
            None => true,
            Some(modified) => observed
                .duration_since(modified)
                .map_or(true, |age| age < RACY_WINDOW),
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only: how many stamps this thread took by PATH (a per-file
    /// `symlink_metadata`, which on Windows opens the file). A full diff
    /// reads stamps from the directory listing instead (GH #623).
    pub(super) static STAMPS_BY_PATH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Test-only: how many whole-file hashes `stamp` took on this thread.
    pub(super) static STAMP_HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn stamp_metadata(path: &Path) -> Option<Stamp> {
    #[cfg(test)]
    STAMPS_BY_PATH.with(|count| count.set(count.get() + 1));
    #[cfg(feature = "test-faults")]
    if crate::file_kind::is_graph_text_path(path) {
        crate::cost_counters::stamp_by_path();
    } else {
        crate::cost_counters::asset_stamp_by_path();
    }
    stamp_from_metadata(&fs::symlink_metadata(path).ok()?)
}

/// The one stamp producer per platform. `metadata` must not follow symlinks:
/// `fs::symlink_metadata(path)` and `DirEntry::metadata()` both qualify. On
/// Windows the latter comes from the directory listing (FindNextFileW), so
/// stamping a walk's entries opens no file (GH #623; spec §1 stamp, Q8).
pub(crate) fn stamp_from_metadata(metadata: &fs::Metadata) -> Option<Stamp> {
    if !metadata.file_type().is_file() {
        return None;
    }
    #[cfg(unix)]
    let (identity, changed) = {
        use std::os::unix::fs::MetadataExt;
        (
            ((metadata.dev() as u128) << 64) | metadata.ino() as u128,
            metadata.ctime() as i128 * 1_000_000_000 + metadata.ctime_nsec() as i128,
        )
    };
    #[cfg(windows)]
    let (identity, changed) = {
        use std::os::windows::fs::MetadataExt;
        (
            metadata.creation_time() as u128,
            metadata.last_write_time() as i128,
        )
    };
    #[cfg(not(any(unix, windows)))]
    let (identity, changed) = (
        metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos()),
        0,
    );
    Some(Stamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
        identity,
        changed,
        rev: None,
    })
}
