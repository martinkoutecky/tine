//! Folder scan: the metadata of a file the listing just named.

use std::fs;
use std::path::Path;

thread_local! {
    /// Run before a listed file's metadata read and before its NotFound
    /// re-check: tests remove or restore the file inside that window.
    #[allow(clippy::type_complexity)]
    pub(crate) static SCAN_HOOKS: std::cell::RefCell<(
        Option<Box<dyn FnOnce(&Path)>>,
        Option<Box<dyn FnOnce(&Path)>>,
    )> = const { std::cell::RefCell::new((None, None)) };
}

/// A name listed a moment ago and now absent was removed or is being
/// replaced (a sync tool that archives the old version before moving the new
/// one in): look once more, then treat it as gone (`None`), never as
/// unreadable. The watcher delivers whatever arrives next.
pub(crate) fn listed_metadata(
    entry: &fs::DirEntry,
    path: &Path,
) -> Option<std::io::Result<fs::Metadata>> {
    #[cfg(test)]
    SCAN_HOOKS.with(|hooks| {
        if let Some(hook) = hooks.borrow_mut().0.take() {
            hook(path);
        }
    });
    match entry.metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(test)]
            SCAN_HOOKS.with(|hooks| {
                if let Some(hook) = hooks.borrow_mut().1.take() {
                    hook(path);
                }
            });
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
