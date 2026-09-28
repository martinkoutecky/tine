//! Shared atomic writes for store and device files.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEW_TMP_SEQ: AtomicU64 = AtomicU64::new(0);
static WRITE_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) fn atomic_write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let seq = NEW_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.{}.{}.new.tmp", std::process::id(), seq));
    let res = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        super::no_replace::move_file_noreplace(&tmp, path)?;
        super::directory_durability::sync_directory_entry(dir)?;
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

pub(crate) fn atomic_write_with_check(
    path: &Path,
    bytes: &[u8],
    check: impl FnOnce() -> io::Result<()>,
    on_write: impl FnOnce(),
    on_file_sync: impl FnOnce(),
    on_dir_sync: impl FnOnce(),
) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let seq = WRITE_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.{}.{seq}.tmp", std::process::id()));
    let res = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        on_write();
        file.sync_all()?;
        on_file_sync();
        drop(file);
        check()?;
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    } else {
        super::directory_durability::sync_directory_entry(dir)?;
        on_dir_sync();
    }
    res
}
