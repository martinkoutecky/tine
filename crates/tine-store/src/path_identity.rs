//! Resolve an existing filesystem path for graph identity and containment.
//! Prefer the filesystem's canonical answer; when a volume cannot provide it,
//! return an absolute spelling only after proving the entry exists and the
//! fallback path contains no symlink or Windows reparse point. Cost is O(path
//! components) on fallback. A missing or unsafe fallback returns the original
//! canonicalization error. Distinct spellings may identify one physical root.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) fn canonical_existing_path(path: &Path) -> io::Result<PathBuf> {
    canonical_existing_path_with(path, |path| fs::canonicalize(path))
}

pub(crate) fn canonical_existing_path_with(
    path: &Path,
    canonicalize: impl FnOnce(&Path) -> io::Result<PathBuf>,
) -> io::Result<PathBuf> {
    match canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(original) => {
            let Ok(absolute) = std::path::absolute(path) else {
                return Err(original);
            };
            if fs::metadata(&absolute).is_err() || has_link_component(&absolute) {
                return Err(original);
            }
            Ok(absolute)
        }
    }
}

fn has_link_component(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        let Ok(metadata) = fs::symlink_metadata(ancestor) else {
            return true;
        };
        if metadata.file_type().is_symlink() {
            return true;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return true;
            }
        }
        false
    })
}
