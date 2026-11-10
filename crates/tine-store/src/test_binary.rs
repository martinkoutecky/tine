//! The running test binary, for tests that start it again as a child process
//! (to kill it, to give it its own environment, to cap its memory).
//!
//! `std::env::current_exe()` on Linux and Android returns the text of the
//! `/proc/self/exe` link. Once the file at that path is replaced while the
//! tests run (another build relinking the same test binary in a shared
//! target directory), the text reads `<path> (deleted)` and spawning it fails
//! with `NotFound`. Executing the link itself opens the running image, which
//! stays valid. macOS, iOS and Windows have no such link: there the path is
//! the one the binary was started from.
//!
//! `tests/own_test_binary.rs` pins both: the replaced-binary child still
//! starts, and no test spawns `current_exe()` itself.

use std::path::PathBuf;

/// The program that runs this test binary again.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn own_test_binary() -> PathBuf {
    PathBuf::from("/proc/self/exe")
}

/// The program that runs this test binary again.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "windows"))]
pub fn own_test_binary() -> PathBuf {
    std::env::current_exe().expect("locate the running test binary")
}
