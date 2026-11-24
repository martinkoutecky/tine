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
    /// A memory file's stamp (`FileSource::Memory`): its carried time, no
    /// length or identity (a document's bytes exist only while it is read).
    /// Only the watcher compares lengths, and nothing watches a memory graph.
    pub(crate) fn in_memory(modified: Option<SystemTime>) -> Stamp {
        Stamp {
            modified,
            len: 0,
            identity: 0,
            changed: 0,
            rev: None,
        }
    }

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

    /// Whether `other` observed the same file identity (Unix dev/inode,
    /// Windows creation time), whatever its times and size.
    pub(crate) fn same_identity(&self, other: &Stamp) -> bool {
        self.identity == other.identity
    }

    /// How `actual` differs from this stamp, for the diagnostics of a failed
    /// `same_metadata` publication check (GH #811): deltas only, no values.
    pub(crate) fn change_to(&self, actual: &Stamp) -> crate::storage_diag::StampChange {
        let delta = |expected: i128, actual: i128| {
            i64::try_from(actual - expected).unwrap_or(if actual > expected {
                i64::MAX
            } else {
                i64::MIN + 1
            })
        };
        let nanos = |time: Option<SystemTime>| {
            time.map(|time| match time.duration_since(SystemTime::UNIX_EPOCH) {
                Ok(after) => after.as_nanos() as i128,
                Err(before) => -(before.duration().as_nanos() as i128),
            })
        };
        crate::storage_diag::StampChange {
            modified_ns: (self.modified != actual.modified).then(|| {
                match (nanos(self.modified), nanos(actual.modified)) {
                    (Some(expected), Some(actual)) => delta(expected, actual),
                    // One side has no time: a change with no measurable delta.
                    _ => 0,
                }
            }),
            size: (self.len != actual.len).then(|| delta(self.len.into(), actual.len.into())),
            identity_device: self.identity >> 64 != actual.identity >> 64,
            identity_file: self.identity as u64 != actual.identity as u64,
            change_time_ns: (self.changed != actual.changed)
                .then(|| delta(self.changed, actual.changed)),
        }
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
    pub(crate) static STAMPS_BY_PATH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Test-only: how many whole-file hashes `stamp` took on this thread.
    pub(super) static STAMP_HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn stamp_metadata(path: &Path) -> Option<Stamp> {
    stamp_from_metadata(&path_metadata(path, false).ok()?)
}

/// The metadata a stamp by path reads, counted: `symlink_metadata`, or with
/// `follow` the followed path's (the config's observation, which reads what
/// the config read reads).
pub(super) fn path_metadata(path: &Path, follow: bool) -> std::io::Result<fs::Metadata> {
    #[cfg(test)]
    STAMPS_BY_PATH.with(|count| count.set(count.get() + 1));
    #[cfg(feature = "test-faults")]
    if crate::file_kind::is_graph_text_path(path) {
        crate::cost_counters::stamp_by_path();
    } else {
        crate::cost_counters::asset_stamp_by_path();
    }
    if follow {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
}

/// What the config read finds, as the watcher observed it
/// (`watch::config_stamp`), by the read's own classification
/// (`model::config_found`): the whole outcome a reload compares, so a change
/// between two outcomes without a revision (a `logseq` file blocking the
/// read, then gone) is seen as one.
#[derive(Clone, Debug)]
pub(crate) enum ConfigStamp {
    /// A file read through: its stamp, with the revision of its bytes.
    File(Stamp),
    /// No file there, a dangling link included.
    Absent,
    /// The read fails, by its error's kind: under a file (`NotADirectory`),
    /// a directory there (`IsADirectory`), bytes that cannot be read.
    Unreadable(std::io::ErrorKind),
}

impl ConfigStamp {
    /// What a stamp observed by a file's own write (`Core::note_own`)
    /// says: a file, or none there.
    pub(crate) fn of_file(stamp: Option<Stamp>) -> Self {
        stamp.map_or(Self::Absent, Self::File)
    }

    pub(crate) fn file(&self) -> Option<&Stamp> {
        match self {
            Self::File(stamp) => Some(stamp),
            Self::Absent | Self::Unreadable(_) => None,
        }
    }

    pub(crate) fn rev(&self) -> Option<&FileRev> {
        self.file().and_then(|stamp| stamp.rev.as_ref())
    }

    /// Whether a config read would find what it found at `other`: the same
    /// revision, both absent, or the same kind of failure.
    pub(crate) fn same_read(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::File(a), Self::File(b)) => a.rev == b.rev,
            (Self::Absent, Self::Absent) => true,
            (Self::Unreadable(a), Self::Unreadable(b)) => a == b,
            _ => false,
        }
    }
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
