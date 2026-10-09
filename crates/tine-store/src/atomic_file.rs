//! Shared atomic writes for store and device files.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::platform_step::at;

static NEW_TMP_SEQ: AtomicU64 = AtomicU64::new(0);
static WRITE_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The longest file name every Tine platform accepts: 255 bytes on Linux,
/// Android and macOS; Windows counts 255 UTF-16 units, never more than the
/// UTF-8 byte count.
const NAME_MAX_BYTES: usize = 255;
/// A temp name may always use this many bytes, even beside a short target.
const TEMP_NAME_FLOOR: usize = 100;

/// Same-directory temp for `path`: `.{name}.{pid}.{seq}{tag}.tmp`. Its name is
/// never longer than `max(target name, TEMP_NAME_FLOOR)` bytes, so the temp fits
/// whenever the target does (C3W W1 / L04: the name + ~20 bytes made every save
/// of a 231–255-byte page name, e.g. an 80-CJK-char title OG writes in place,
/// fail with ENAMETOOLONG). When shortening, the stem is cut at a char boundary
/// and a short extension is kept, so `watch.rs::atomic_temp` still recognizes a
/// page temp; uniqueness comes from pid + seq alone. The only temp-name format
/// in tine-store for a user-named target (C3Y Y1; guarded by
/// `tests/c3y_derived_names_guard.rs`).
pub(crate) fn temp_path(path: &Path, seq: u64, tag: &str) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let suffix = format!(".{}.{seq}{tag}.tmp", std::process::id());
    let limit = fname.len().max(TEMP_NAME_FLOOR).min(NAME_MAX_BYTES);
    let (stem, ext) = split_short_ext(fname);
    dir.join(fit_name(".", stem, &format!("{ext}{suffix}"), limit))
}

/// A trash or conflict-copy name `{prefix}{name}` (prefix = stamp + reason)
/// that fits NAME_MAX whenever `name` itself does: the stem is cut at a char
/// boundary and its extension kept, so the copy is still recognized by kind
/// and recoverable; the unique stamp in `prefix` survives intact (C3Y Y2).
pub(crate) fn prefixed_name(prefix: &str, name: &str) -> String {
    let (stem, ext) = split_short_ext(name);
    fit_name(prefix, stem, ext, NAME_MAX_BYTES)
}

/// A collision candidate `{stem}{mark}{ext}` (e.g. `_1`) that fits NAME_MAX
/// whenever the uncollided name does (C3Y Y2).
pub(crate) fn marked_name(stem: &str, mark: &str, ext: &str) -> String {
    fit_name("", stem, &format!("{mark}{ext}"), NAME_MAX_BYTES)
}

/// A probe of a derived name (the `.md`/`.org` twin) that the filesystem
/// reports absent or unable to exist — a 255-byte `.md` page's `.org` twin is
/// 256 bytes — found nothing (C3Y Y3: the twin probe used to fail the create).
pub(crate) fn names_nothing(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::InvalidFilename
    )
}

/// Split at a short (at most 10-byte) extension a shortened name keeps.
fn split_short_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && name.len() - dot <= 10 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// `{prefix}{stem}{tail}`, with `stem` cut at a char boundary so the whole is
/// at most `limit` bytes. The one shortening rule for every file name tine-store
/// derives from a user's file name.
fn fit_name(prefix: &str, stem: &str, tail: &str, limit: usize) -> String {
    let room = limit.saturating_sub(prefix.len() + tail.len());
    let mut cut = room.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{prefix}{}{tail}", &stem[..cut])
}

/// Shared replace-allowed platform rename (rename(2) / MoveFileExW with
/// REPLACE_EXISTING). The caller owns its guard and directory durability.
pub(crate) fn rename_replace(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let src: Vec<_> = src.as_os_str().encode_wide().chain(Some(0)).collect();
        let dst: Vec<_> = dst.as_os_str().encode_wide().chain(Some(0)).collect();
        let result = unsafe {
            MoveFileExW(
                src.as_ptr(),
                dst.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        (result != 0)
            .then_some(())
            .ok_or_else(io::Error::last_os_error)
    }
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        fs::rename(src, dst)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "windows"
    )))]
    {
        let _ = (src, dst);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "replace unavailable on this target",
        ))
    }
}

/// The existing atomic-write protocol split at its synced-temp barrier. The
/// caller retains its base guard and page lock through publication and sync.
pub(crate) struct PreparedWrite {
    temporary: PathBuf,
    target: PathBuf,
}

impl PreparedWrite {
    pub(crate) fn new(path: &Path, bytes: &[u8]) -> io::Result<Self> {
        Self::with_hooks(path, bytes, || {}, || {})
    }

    fn with_hooks(
        path: &Path,
        bytes: &[u8],
        on_write: impl FnOnce(),
        on_file_sync: impl FnOnce(),
    ) -> io::Result<Self> {
        let temporary = temp_path(path, WRITE_TMP_SEQ.fetch_add(1, Ordering::Relaxed), "");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(at("create temporary file"))?;
        let prepared = Self {
            temporary,
            target: path.to_path_buf(),
        };
        file.write_all(bytes).map_err(at("write temporary file"))?;
        #[cfg(test)]
        TEMP_WRITES.with(|count| {
            let (files, written) = count.get();
            count.set((files + 1, written + bytes.len() as u64));
        });
        on_write();
        sync_file_handle(&file).map_err(at("fsync temporary file"))?;
        on_file_sync();
        Ok(prepared)
    }

    pub(crate) fn publish(&self, create: bool) -> io::Result<()> {
        if create {
            super::no_replace::move_file_noreplace(&self.temporary, &self.target)
        } else {
            rename_replace(&self.temporary, &self.target)
                .map_err(at("rename temporary file over target"))
        }
    }
}

impl Drop for PreparedWrite {
    fn drop(&mut self) {
        // Only this protocol's unpublished scratch file; never the target.
        let _ = fs::remove_file(&self.temporary);
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static FAIL_FILE_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(crate) static FILE_SYNCS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    pub(crate) static TEMP_WRITES: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

pub(crate) fn atomic_write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = temp_path(path, NEW_TMP_SEQ.fetch_add(1, Ordering::Relaxed), ".new");
    let res = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(at("create temporary file"))?;
        file.write_all(bytes).map_err(at("write temporary file"))?;
        file.sync_all().map_err(at("fsync temporary file"))?;
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

/// Make the bytes already at `path` durable: fsync the file, then its
/// directory entry. A save whose bytes equal the disk reports success without
/// a rename, and success retires the editor's crash copy, so those bytes must
/// be as durable as a rename would have made them. Scenario: another program
/// (Syncthing) writes the user's unsaved text without fsync, Tine's save finds
/// equal bytes, and a power cut then reverts the file (storage.qnt mutant MQ).
/// Windows needs a writable handle for FlushFileBuffers.
pub(crate) fn sync_existing(path: &Path) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    sync_file_bytes(path)?;
    super::directory_durability::sync_directory_entry(dir)
}

/// File part of the existing durability recipe, also used for bytes moved
/// into trash: an external R1 writer need not have flushed those bytes.
pub(crate) fn sync_file_bytes(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    let file = fs::OpenOptions::new().write(true).open(path);
    #[cfg(not(windows))]
    let file = fs::File::open(path);
    file.and_then(|file| sync_file_handle(&file))
        .map_err(at("fsync unchanged file"))
}

fn sync_file_handle(file: &fs::File) -> io::Result<()> {
    #[cfg(test)]
    FILE_SYNCS.with(|count| count.set(count.get() + 1));
    #[cfg(test)]
    if FAIL_FILE_SYNC.with(|fail| fail.replace(false)) {
        return Err(io::Error::other("injected file sync failure"));
    }
    file.sync_all()
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
    let prepared = PreparedWrite::with_hooks(path, bytes, on_write, on_file_sync)?;
    check()?;
    prepared.publish(false)?;
    super::directory_durability::sync_directory_entry(dir)?;
    on_dir_sync();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_name_never_outgrows_a_long_target_and_keeps_the_page_extension() {
        for title in ["a".repeat(250), "漢".repeat(84), "漢".repeat(40)] {
            let target = Path::new("/g/pages").join(format!("{title}.md"));
            let name_len = target.file_name().unwrap().len();
            for tag in ["", ".new"] {
                let tmp = temp_path(&target, u64::MAX, tag);
                let tmp_name = tmp.file_name().unwrap().to_str().unwrap();
                assert!(
                    tmp_name.len() <= name_len.max(TEMP_NAME_FLOOR),
                    "{tmp_name}"
                );
                assert!(tmp_name.starts_with('.') && tmp_name.ends_with(".tmp"));
                assert!(tmp_name.contains(".md."), "{tmp_name}");
                assert_eq!(tmp.parent(), target.parent());
            }
        }
        let short = temp_path(Path::new("/g/pages/A.md"), 7, ".import");
        assert_eq!(
            short.file_name().unwrap().to_str().unwrap(),
            format!(".A.md.{}.7.import.tmp", std::process::id())
        );
        let short = temp_path(Path::new("/g/pages/A.md"), 7, "");
        assert_eq!(
            short.file_name().unwrap().to_str().unwrap(),
            format!(".A.md.{}.7.tmp", std::process::id())
        );
    }

    #[test]
    fn trash_and_collision_names_fit_and_keep_their_stamp_and_extension() {
        for name in [
            format!("{}.png", "c".repeat(251)),
            format!("{}.md", "漢".repeat(84)),
        ] {
            let trash = prefixed_name("1759000000000-12__tx-old__", &name);
            assert!(trash.len() <= NAME_MAX_BYTES, "{trash}");
            assert!(trash.starts_with("1759000000000-12__tx-old__"));
            assert_eq!(Path::new(&trash).extension(), Path::new(&name).extension());
            let (stem, ext) = split_short_ext(&name);
            let marked = marked_name(stem, "_12", ext);
            assert!(marked.len() <= NAME_MAX_BYTES && marked.ends_with(&format!("_12{ext}")));
        }
        assert_eq!(prefixed_name("s__", "A.md"), "s__A.md");
        assert_eq!(marked_name("A", "_1", ".pdf"), "A_1.pdf");
    }
}
