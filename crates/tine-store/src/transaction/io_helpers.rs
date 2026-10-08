//! Transaction I/O error mapping and namespace synchronization.

use std::fs;
use std::io;
use std::path::Path;

#[cfg(all(feature = "test-faults", unix))]
use super::{fault, FaultPoint};
use super::{IoError, Refusal, Why};
use crate::store::{FileId, FileRev, Store};

pub(super) fn failed(error: io::Error) -> Why {
    Why::Failed(error.into())
}

pub(super) fn publication_path_error(error: &Why) -> IoError {
    io::Error::other(format!("{error:?}")).into()
}

/// An undo step whose file id no longer resolves to a path. No I/O; O(1).
pub(super) fn unresolved_undo_path(error: &impl std::fmt::Debug) -> IoError {
    io::Error::new(io::ErrorKind::InvalidInput, format!("{error:?}")).into()
}

pub(super) fn content_refusal(error: io::Error) -> Why {
    if error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<std::str::Utf8Error>())
        .is_some()
    {
        return Why::Refused(Refusal::Undecodable);
    }
    Why::Refused(Refusal::InvalidTarget(format!(
        "page content cannot be parsed safely: {error}"
    )))
}

/// Reading a directory fails with EISDIR on Unix but ERROR_ACCESS_DENIED on
/// Windows. Report both as `IsADirectory` so a caller that treats an occupying
/// directory as "name taken" (the Guide copy) behaves the same on every
/// platform instead of failing with "Access is denied" on Windows.
pub(super) fn directory_read_error(error: io::Error, path: &Path) -> io::Error {
    if error.kind() != io::ErrorKind::IsADirectory
        && fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
    {
        io::Error::new(
            io::ErrorKind::IsADirectory,
            format!("{}: is a directory ({error})", path.display()),
        )
    } else {
        error
    }
}

pub(super) fn failed_trash_dir(error: io::Error, parent: &Path) -> Why {
    let display = if error.kind() == io::ErrorKind::NotADirectory
        && parent.ends_with(Path::new("logseq/.tine-trash/assets"))
    {
        Path::new("logseq/.tine-trash/assets")
    } else {
        parent
    };
    Why::Failed(IoError {
        kind: error.kind(),
        message: format!(
            "could not create trash directory {}: {error}",
            display.display()
        ),
        operation: None,
        os_error: None,
    })
}

pub(super) fn sync_move_dirs(store: &Store, source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(all(feature = "test-faults", unix))]
    if fault(store, FaultPoint::DirectorySyncIo) {
        crate::directory_durability::fail_next_sync();
    }
    #[cfg(not(all(feature = "test-faults", unix)))]
    let _ = store;
    for parent in source.parent().into_iter().chain(
        destination
            .parent()
            .filter(|parent| Some(*parent) != source.parent()),
    ) {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::fsync();
        crate::directory_durability::sync_directory_entry(parent)?;
    }
    Ok(())
}

pub(super) fn disk_rev(path: &Path) -> Option<FileRev> {
    fs::read(path).ok().map(|bytes| FileRev::from_bytes(&bytes))
}

pub(super) fn collision(file: &FileId, error: io::Error, path: &Path) -> Why {
    if error.kind() == io::ErrorKind::AlreadyExists {
        Why::Conflict {
            file: file.clone(),
            disk: disk_rev(path),
        }
    } else {
        failed(error)
    }
}
