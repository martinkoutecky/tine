//! §5.4 racy-observation helpers for the watcher's full diff.
use super::{stamp_metadata, Stamp};
use crate::store::FileRev;
use std::path::Path;

#[cfg(test)]
thread_local! {
    pub(super) static HASH_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    // A denied/offline open, before any bytes can be read. Path-scoped so
    // unrelated files still exercise the real filesystem.
    pub(super) static HASH_OPEN_ERROR: std::cell::RefCell<Option<(std::path::PathBuf, std::io::ErrorKind)>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn hash_revision(path: &Path) -> std::io::Result<FileRev> {
    #[cfg(test)]
    HASH_ATTEMPTS.with(|count| count.set(count.get() + 1));
    #[cfg(test)]
    if let Some(kind) = HASH_OPEN_ERROR.with(|fault| {
        fault
            .borrow()
            .as_ref()
            .and_then(|(failed, kind)| (failed == path).then_some(*kind))
    }) {
        return Err(std::io::Error::new(kind, "injected open failure"));
    }
    FileRev::from_file(path)
}

/// Hashes `path` for the observation `value`, re-statting afterwards: a write
/// landing between the stat and the read (external-editor race) would pair an
/// old stamp with new bytes and publish a spurious second change once the
/// next diff sees the new stamp. On a moved stat the fresh one is adopted and
/// hashed again; returns whether it moved. A stamp still moving after that
/// returns a retryable error. Read errors retain their I/O class so a settled
/// permission refusal does not become an untyped hash failure.
pub(super) fn hash_settled(path: &Path, value: &mut Stamp) -> std::io::Result<bool> {
    let mut moved = false;
    for _ in 0..3 {
        let revision = hash_revision(path);
        let Some(fresh) = stamp_metadata(path) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "file disappeared during read",
            ));
        };
        if fresh.same_metadata(value) {
            value.rev = Some(revision?);
            return Ok(moved);
        }
        moved = true;
        *value = fresh;
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "file changed during read",
    ))
}
