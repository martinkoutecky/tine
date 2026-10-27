//! Folder scan: per-entry reads and their test seams.

use std::fs;
use std::path::Path;

/// A listed file's metadata. `None` means the name vanished between listing
/// and this read and is still absent (removed), or is no longer a file: skip
/// it. A name listed a moment ago and now absent was removed or is being
/// replaced (a sync tool that archives the old version before moving the new
/// one in), so look once more and treat it as gone, never as unreadable. The
/// watcher delivers whatever arrives next.
pub(super) fn listed_file_metadata(
    entry: &fs::DirEntry,
    path: &Path,
) -> Option<std::io::Result<fs::Metadata>> {
    #[cfg(test)]
    run_hook(0, path);
    match entry.metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(test)]
            run_hook(1, path);
            match fs::symlink_metadata(path) {
                Ok(meta) if meta.is_file() => Some(Ok(meta)),
                Ok(_) => None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => Some(Err(error)),
            }
        }
        other => Some(other),
    }
}

/// Injected failure for listing `dir`.
#[cfg(test)]
pub(super) fn forced_list_failure(root: &Path, dir: &Path) -> Option<std::io::Error> {
    SCAN_FAULTS.with(|faults| {
        let rel = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy();
        (faults.borrow().1.as_deref() == Some(rel.as_ref())).then(|| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected list failure",
            )
        })
    })
}

#[cfg(not(test))]
pub(super) fn forced_list_failure(_root: &Path, _dir: &Path) -> Option<std::io::Error> {
    None
}

/// Injected failure for reading the type of the entry named `rel`.
#[cfg(test)]
pub(super) fn forced_stat_failure(rel: &str) -> Option<std::io::Error> {
    SCAN_FAULTS.with(|faults| {
        (faults.borrow().0.as_deref() == Some(rel)).then(|| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected stat failure",
            )
        })
    })
}

#[cfg(not(test))]
pub(super) fn forced_stat_failure(_rel: &str) -> Option<std::io::Error> {
    None
}

#[cfg(test)]
fn run_hook(slot: usize, path: &Path) {
    let hook = SCAN_HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        if slot == 0 {
            hooks.0.take()
        } else {
            hooks.1.take()
        }
    });
    if let Some(hook) = hook {
        hook(path);
    }
}

#[cfg(test)]
thread_local! {
    /// (entry name whose type read fails, directory whose listing fails).
    pub(super) static SCAN_FAULTS: std::cell::RefCell<(Option<String>, Option<String>)> =
        const { std::cell::RefCell::new((None, None)) };
    /// Run before a listed file's metadata read and before its NotFound
    /// re-check: tests remove or restore the file inside that window.
    #[allow(clippy::type_complexity)]
    pub(super) static SCAN_HOOKS: std::cell::RefCell<(
        Option<Box<dyn FnOnce(&Path)>>,
        Option<Box<dyn FnOnce(&Path)>>,
    )> = const { std::cell::RefCell::new((None, None)) };
}
