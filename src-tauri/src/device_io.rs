//! Durable writes for the device settings file outside graph roots.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

// Compile the one audited no-replace primitive in both crates without adding
// a graph-independent device path to tine-store's public API.
#[allow(dead_code)]
#[path = "../../crates/tine-store/src/no_replace.rs"]
mod no_replace;

/// Device source errors remain distinct so the command can preserve its wire text.
#[derive(Debug)]
pub(crate) enum DeviceAssetImportError {
    Name(String),
    Io(io::Error),
}

/// Open a caller-selected device file once, then stream it through the graph
/// asset transaction. The asset client owns filename and collision policy.
pub(crate) fn import_asset_from_path(
    store: &tine_store::Store,
    path: &str,
    name: Option<&str>,
) -> Result<String, DeviceAssetImportError> {
    let source_filename = Path::new(path).file_name().and_then(|value| value.to_str());
    let chosen = tine_graph_features::assets::choose_import_name(source_filename, name)
        .map_err(DeviceAssetImportError::Name)?;
    let source = fs::File::open(path).map_err(DeviceAssetImportError::Io)?;
    tine_graph_features::assets::import_asset(
        store,
        &chosen,
        tine_store::Content::Stream {
            source,
            max_bytes: u64::MAX,
        },
    )
    .map_err(DeviceAssetImportError::Io)
}

#[cfg(test)]
mod asset_import_tests {
    use super::*;

    #[test]
    fn import_path_selects_name_before_open_and_streams_once() {
        let root = std::env::temp_dir().join(format!("tine-device-import-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for area in ["pages", "journals", "assets"] {
            fs::create_dir_all(root.join(area)).unwrap();
        }
        let source = root.with_extension("source.bin");
        fs::write(&source, b"media").unwrap();
        let store = tine_store::Store::open(&root, tine_store::OpenOptions::default())
            .unwrap()
            .0;
        assert!(
            matches!(import_asset_from_path(&store, source.to_str().unwrap(), Some("../bad")),
            Err(DeviceAssetImportError::Name(message)) if message == "bad asset name")
        );
        assert_eq!(
            import_asset_from_path(&store, source.to_str().unwrap(), Some("kept.bin")).unwrap(),
            "kept.bin"
        );
        assert_eq!(fs::read(root.join("assets/kept.bin")).unwrap(), b"media");
        fs::remove_file(source).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

/// Atomically move one file without ever replacing an existing destination.
/// Platform-native no-replace rename semantics ensure the source name and inode
/// cannot be swapped between a check and an unlink.
pub(crate) fn move_file_noreplace(src: &Path, dest: &Path) -> io::Result<()> {
    no_replace::move_file_noreplace(src, dest)
}

/// Atomically publish a newly-created file without clobbering a destination that
/// appeared after the caller's collision check. The payload is fsynced in a
/// same-directory temp, then atomically renamed into the final name only if absent.
pub(crate) fn atomic_write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.{}.{}.new.tmp", std::process::id(), seq));
    let res = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        move_file_noreplace(&tmp, path)?;
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Atomic write: write to a temp file in the same directory, then rename. The
/// temp name is unique per write (pid + sequence) so two concurrent writers to
/// the same path (e.g. an autosave and a highlight/rename rewrite) can't truncate
/// each other's temp; the rename is still atomic. The temp is removed if the
/// write fails, so a unique name never leaks an orphan behind.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.{}.{seq}.tmp", std::process::id()));
    let res = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp); // never leave a temp behind on failure
    } else {
        // Persist the rename itself: fsync the directory so a crash right after the
        // write can't lose the new directory entry (the rename) on some
        // filesystems. Best-effort — not all platforms allow fsync on a dir.
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
    res
}

/// Read–modify–write a small text file (config.edn, device settings) under a lock,
/// committed via [`atomic_write`]. The ONE guarded path every settings writer goes
/// through, so the discipline is uniform rather than re-derived per call site:
///   - a MISSING file is the empty document `{}`, but any OTHER read error
///     (permission, NFS stale handle, transient I/O) ABORTS — otherwise `edit` would
///     rebuild the whole file from `{}` and destroy every other key (audit H2);
///   - the `lock` serializes concurrent writers to the same logical file so a
///     read-modify-write can't clobber a concurrent one (audit M1/M2);
///   - `edit` returns the new full contents, or an `Err` to abort without writing;
///   - the commit is atomic (temp + fsync + rename), so a crash can't truncate it.
pub(crate) fn atomic_update(
    path: &Path,
    lock: &std::sync::Mutex<()>,
    edit: impl Fn(&str) -> io::Result<String>,
) -> io::Result<()> {
    atomic_update_with_hooks(path, lock, edit, |_| {}, |_| {})
}

fn atomic_update_with_hooks(
    path: &Path,
    lock: &std::sync::Mutex<()>,
    edit: impl Fn(&str) -> io::Result<String>,
    before_recheck: impl Fn(usize),
    before_publish: impl Fn(usize),
) -> io::Result<()> {
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for attempt in 0..4 {
        let baseline = match fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let next = edit(baseline.as_deref().unwrap_or("{}\n"))?;
        // CONFIG_LOCK serializes Tine writers, but Logseq/Syncthing do not take
        // it. Re-read immediately before publish and retry the key-local edit on
        // their new bytes instead of overwriting an external update with our stale
        // full-file copy.
        before_recheck(attempt);
        let current = match fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if current != baseline {
            continue;
        }
        before_publish(attempt);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let published = if baseline.is_none() {
            atomic_write_new(path, next.as_bytes())
        } else {
            atomic_write(path, next.as_bytes())
        };
        match published {
            Ok(()) => return Ok(()),
            Err(error) if baseline.is_none() && error.kind() == io::ErrorKind::AlreadyExists => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "config changed repeatedly during update",
    ))
}
