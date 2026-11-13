//! Atomic allocation for legacy fixtures that own a PathBuf rather than a guard.
//!
//! Existing fixtures retain their cleanup policy: callers remove the directory
//! explicitly, or keep it for inspection. New scoped fixtures should hold a
//! `tempfile::TempDir` instead. Allocation does no graph I/O and normally takes
//! one mkdir; tempfile retries atomically if a candidate already exists.

use std::path::PathBuf;

/// Create an empty, unique directory, leaving cleanup to the fixture's owner.
pub(crate) fn fresh_dir(prefix: &str) -> PathBuf {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("allocate unique test directory")
        .keep()
}
