//! Durable writes for the device settings file outside graph roots.

use std::fs;
use std::io;
use std::path::Path;

// Compile the one audited no-replace primitive in both crates without adding
// a graph-independent device path to tine-store's public API.
#[allow(dead_code)]
#[path = "../../crates/tine-store/src/no_replace.rs"]
mod no_replace;

#[path = "../../crates/tine-store/src/atomic_file.rs"]
mod atomic_file;
#[allow(dead_code)]
#[path = "../../crates/tine-store/src/platform_step.rs"]
mod platform_step;
use tine_store::directory_durability;

/// Open a caller-selected device file only if it is a regular file (I-22).
/// Threat scenario (imported content): a dropped, pasted or picked path may
/// name a FIFO, socket or device node, and a blocking `open` of a FIFO with no
/// writer hangs the synchronous command before any later validation runs.
/// Path metadata refuses a non-regular file before open; the open is
/// nonblocking on Unix so a path swapped for a FIFO after that check cannot
/// hang either; opened-handle metadata rechecks the race. `O_NONBLOCK` has no
/// effect on reads of a regular file.
fn open_regular_file(path: &Path) -> io::Result<fs::File> {
    let not_a_file = || io::Error::new(io::ErrorKind::InvalidInput, "not a file");
    if !fs::metadata(path)?.is_file() {
        return Err(not_a_file());
    }
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::File::options()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?
    };
    #[cfg(not(unix))]
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(not_a_file());
    }
    Ok(file)
}

/// Reads a caller-selected regular file ([`open_regular_file`]); the caller
/// enforces graph scope. Reads never more than `max` bytes: the read stops at
/// `max + 1` so a file that grows (or lies about its length) after the check
/// cannot allocate past the limit (I-22).
pub(crate) fn read_regular_file_bounded(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = open_regular_file(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if meta.len() > max {
        return Err("image too large".into());
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("image too large".into());
    }
    Ok(bytes)
}

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
    let source = open_regular_file(Path::new(path)).map_err(DeviceAssetImportError::Io)?;
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
mod tests {
    use super::*;

    #[test]
    fn atomic_update_retries_on_external_change_without_losing_it() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let path = dir.join("config.edn");
        fs::write(&path, "{:base 1}\n").unwrap();
        let lock = std::sync::Mutex::new(());
        let injected = std::sync::atomic::AtomicBool::new(false);
        atomic_update_with_hooks(
            &path,
            &lock,
            |content| Ok(content.replace('}', " :mine 3}")),
            |_| {
                if !injected.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    fs::write(&path, "{:base 1 :external 2}\n").unwrap();
                }
            },
            |_| {},
        )
        .unwrap();
        let final_content = fs::read_to_string(&path).unwrap();
        assert!(final_content.contains(":external 2"));
        assert!(final_content.contains(":mine 3"));
    }

    #[test]
    fn atomic_update_absent_publish_preserves_a_concurrent_creator() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let path = dir.join("config.edn");
        let lock = std::sync::Mutex::new(());
        let injected = std::sync::atomic::AtomicBool::new(false);
        atomic_update_with_hooks(
            &path,
            &lock,
            |content| Ok(content.replace('}', " :mine 3}")),
            |_| {},
            |_| {
                if !injected.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    fs::write(&path, "{:external 2}\n").unwrap();
                }
            },
        )
        .unwrap();
        let final_content = fs::read_to_string(&path).unwrap();
        assert!(final_content.contains(":external 2"));
        assert!(final_content.contains(":mine 3"));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_image_read_refuses_fifo_without_waiting_for_writer() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe.png");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (tx, rx) = mpsc::channel();
        let reader = fifo.clone();
        let worker = std::thread::spawn(move || {
            tx.send(read_regular_file_bounded(&reader, 16)).unwrap();
        });
        let quick = rx.recv_timeout(Duration::from_millis(100));
        if quick.is_err() {
            let _writer = fs::OpenOptions::new().write(true).open(&fifo).unwrap();
        }
        let was_quick = quick.is_ok();
        let result = quick.unwrap_or_else(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap());
        worker.join().unwrap();
        assert_eq!(result.unwrap_err(), "not a file");
        assert!(was_quick, "FIFO read waited for a writer");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_image_read_stops_at_the_limit_even_when_metadata_undercounts() {
        // og 15b K09 (I-22): the local-image read stops at the limit itself.
        // A procfs file reports length 0 to metadata but yields more bytes,
        // the deterministic stand-in for a file that grows after the check.
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("grows.png");
        std::os::unix::fs::symlink("/proc/self/status", &image).unwrap();
        assert_eq!(fs::metadata(&image).unwrap().len(), 0);
        assert_eq!(
            read_regular_file_bounded(&image, 16).unwrap_err(),
            "image too large"
        );
        let small = dir.path().join("small.png");
        fs::write(&small, [7u8; 16]).unwrap();
        assert_eq!(read_regular_file_bounded(&small, 16).unwrap(), [7u8; 16]);
        assert_eq!(
            read_regular_file_bounded(dir.path(), 16).unwrap_err(),
            "not a file"
        );
    }

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

    #[cfg(unix)]
    #[test]
    fn asset_import_refuses_fifo_without_waiting_for_writer() {
        // og C (I-22): a dropped `capture.png` that is a FIFO with no writer
        // must be refused before a blocking open, and leave no asset.
        use std::sync::mpsc;
        use std::time::Duration;
        let graph = tempfile::tempdir().unwrap();
        for area in ["pages", "journals", "assets"] {
            fs::create_dir_all(graph.path().join(area)).unwrap();
        }
        let outside = tempfile::tempdir().unwrap();
        let fifo = outside.path().join("capture.png");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let root = graph.path().to_path_buf();
        let path = fifo.to_str().unwrap().to_owned();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let store = tine_store::Store::open(&root, tine_store::OpenOptions::default())
                .unwrap()
                .0;
            let started = std::time::Instant::now();
            let result = import_asset_from_path(&store, &path, None);
            tx.send((result.map_err(|e| format!("{e:?}")), started.elapsed()))
                .unwrap();
        });
        // Store::open may take a while on a loaded machine; the claim is about
        // the import call itself, which the worker times.
        let outcome = rx.recv_timeout(Duration::from_secs(30));
        if outcome.is_err() {
            // Release a hung open so the test thread can finish and report.
            let _writer = fs::OpenOptions::new().write(true).open(&fifo).unwrap();
        }
        let (result, elapsed) = outcome.expect("asset import hung on a FIFO with no writer");
        worker.join().unwrap();
        assert!(result.unwrap_err().contains("not a file"));
        assert!(
            elapsed < Duration::from_secs(5),
            "import waited {elapsed:?}"
        );
        assert_eq!(
            fs::read_dir(graph.path().join("assets")).unwrap().count(),
            0
        );
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
    atomic_file::atomic_write_new(path, bytes)
}

/// Atomic write: write to a temp file in the same directory, then rename. The
/// temp name is unique per write (pid + sequence) so two concurrent writers to
/// the same path (e.g. an autosave and a highlight/rename rewrite) can't truncate
/// each other's temp; the rename is still atomic. The temp is removed if the
/// write fails, so a unique name never leaks an orphan behind.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_file::atomic_write_with_check(path, bytes, || Ok(()), || {}, || {}, || {})
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
        // The supplied file lock serializes Tine writers; external editors and
        // sync services do not take it. Re-read immediately before publish and retry the key-local edit on
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

/// Copy regular files and directories; symlinks and special files are skipped
/// (none of the allowlisted entries contains one, and following one could read
/// outside the released dir).
pub(crate) fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let kind = fs::symlink_metadata(from)?.file_type();
    if kind.is_dir() {
        fs::create_dir_all(to)?;
        for child in fs::read_dir(from)? {
            let child = child?;
            copy_tree(&child.path(), &to.join(child.file_name()))?;
        }
        sync_dir(to)
    } else if kind.is_file() {
        // Reuse the audited device publication path. A copy is one existing
        // file format, and its source is read-only throughout the operation.
        atomic_write_new(to, &fs::read(from)?)?;
        fs::set_permissions(to, fs::metadata(from)?.permissions())
    } else {
        Ok(())
    }
}

/// The one rename: publish a staged entry, or set a Welcome-only dir aside.
pub(crate) fn publish_directory_entry(from: &Path, to: &Path) -> io::Result<()> {
    move_file_noreplace(from, to)?;
    to.parent().map_or(Ok(()), sync_dir)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    tine_store::directory_durability::sync_directory_entry(dir)
}
