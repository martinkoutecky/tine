//! Shared atomic writes for store and device files.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::platform_step::at;

/// The sequence part of every [`temp_path`] this file makes.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

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
/// and a short extension is kept, so [`parse_temp_name`] still finds a page
/// temp's extension; uniqueness comes from pid + seq alone. The only temp-name
/// format in tine-store for a user-named target (C3Y Y1; guarded by
/// `tests/c3y_derived_names_guard.rs`). The tags in use: `""` (a replacing
/// save), `.new`, `.private`, `.restore`, `.capture`.
pub(crate) fn temp_path(path: &Path, seq: u64, tag: &str) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let suffix = format!(".{}.{seq}{tag}.tmp", std::process::id());
    let limit = fname.len().max(TEMP_NAME_FLOOR).min(NAME_MAX_BYTES);
    let (stem, ext) = split_short_ext(fname);
    dir.join(fit_name(".", stem, &format!("{ext}{suffix}"), limit))
}

/// The one reader of [`temp_path`]'s names: `(target name, tag)` for
/// `.{name}.{pid}.{seq}{tag}.tmp`, with numeric pid and seq and a tag of at
/// most one dot-free component (`""` or `.new`). For a shortened temp the
/// target name is the one the temp carries (the cut stem and its kept
/// extension), not the full name: the cut is not reversible. Each watcher keeps
/// its own folder scope. O(name length), no I/O.
pub(crate) fn parse_temp_name(name: &str) -> Option<(&str, &str)> {
    let stem = name.strip_prefix('.')?.strip_suffix(".tmp")?;
    let numeric = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let (before, last) = stem.rsplit_once('.')?;
    let (before, seq, tag) = if numeric(last) {
        (before, last, "")
    } else {
        let (rest, seq) = before.rsplit_once('.')?;
        (rest, seq, &stem[before.len()..])
    };
    let (target, pid) = before.rsplit_once('.')?;
    (numeric(seq) && numeric(pid) && !target.is_empty()).then_some((target, tag))
}

/// The fixed-form temp `.tine-{kind}-{unique}.tmp`, for scratch that has no
/// user-named target of its own (restore's directory-handle copy, a streamed
/// transaction's stage). Never shortened: `kind` and `unique` are Tine's.
pub(crate) fn fixed_temp_name(kind: &str, unique: &str) -> String {
    format!(".tine-{kind}-{unique}.tmp")
}

/// Whether `name` has [`fixed_temp_name`]'s form. O(name length), no I/O.
pub(crate) fn is_fixed_temp_name(name: &str) -> bool {
    name.starts_with(".tine-") && name.ends_with(".tmp")
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

// GH #811: the mtime reset after a replacing rename on Android is best
// effort; its outcomes are counted process-wide, for the storage
// diagnostics (`androidMtimeReset`, D-20261031-MTIME). Kept here, not in
// `storage_diag`, because the app crate also includes this file.
#[cfg(target_os = "android")]
#[allow(dead_code)]
pub(crate) static ANDROID_MTIME_RESET_SUCCEEDED: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
#[cfg(target_os = "android")]
#[allow(dead_code)]
pub(crate) static ANDROID_MTIME_RESET_FAILED: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// `path`'s mtime without opening it (utimensat, the atime untouched): one
/// FUSE SETATTR on Android shared storage, and no open there.
#[cfg(target_os = "android")]
fn set_modified_at(path: &Path, modified: std::time::SystemTime) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let since = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let times = [
        libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_OMIT,
        },
        libc::timespec {
            tv_sec: since.as_secs() as libc::time_t,
            tv_nsec: since.subsec_nanos() as _,
        },
    ];
    // SAFETY: a NUL-terminated path and two timespecs, as utimensat(2) takes.
    let result = unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            path.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
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
    #[cfg(feature = "test-faults")]
    crate::cost_counters::file_rename();
    #[cfg(target_os = "windows")]
    {
        // std's own call plus MOVEFILE_WRITE_THROUGH, on the shared native
        // path form. Any failure takes std's rename itself, so its long-path
        // handling and ACCESS_DENIED -> FileRenameInfoEx retry stay Beta's
        // behaviour (REVIEW-2b F10).
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let moved = unsafe {
            MoveFileExW(
                native_path(src).as_ptr(),
                native_path(dst).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) != 0
        };
        if moved {
            Ok(())
        } else {
            fs::rename(src, dst)
        }
    }
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        // GH #811: on Android shared storage a rename over a file leaves the
        // replaced file's mtime on the new one (the kernel flushes the
        // replaced FUSE inode's times by node id, and MediaProvider applies
        // them at the path). Put the source's own mtime back, as rename(2)
        // keeps it, so a sync client's rescan sees a same-size edit.
        #[cfg(target_os = "android")]
        let modified = fs::symlink_metadata(src).and_then(|metadata| metadata.modified());
        fs::rename(src, dst)?;
        #[cfg(target_os = "android")]
        {
            // The rename has landed; failing here would report a published
            // file as unwritten. A failure leaves MediaProvider's mtime and
            // is counted in the storage diagnostics (D-20261031-MTIME).
            let reset = modified.and_then(|modified| set_modified_at(dst, modified));
            let counter = if reset.is_ok() {
                &ANDROID_MTIME_RESET_SUCCEEDED
            } else {
                &ANDROID_MTIME_RESET_FAILED
            };
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
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

/// The one Windows path form for Tine's native moves (`rename_replace` and
/// `no_replace::move_windows`; REVIEW-2b-r2 R2), NUL-terminated. A path that is
/// already verbatim, or whose absolute form (`std::path::absolute`, i.e.
/// GetFullPathNameW) is shorter than the 248-unit legacy limit, is passed
/// exactly as spelled, which is what these calls received before. Only a
/// longer one becomes `\\?\` + absolute (`\\.\` → `\\?\`, `\\server` →
/// `\\?\UNC\server`), so the only change is failure → success past MAX_PATH.
/// This is std's `get_long_path` prefix table, not its exact algorithm: std
/// also converts short relative paths and skips GetFullPathNameW for short
/// absolute ones.
#[cfg(windows)]
pub(crate) fn native_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    const LEGACY_MAX_PATH: usize = 248;
    let spelled: Vec<u16> = path.as_os_str().encode_wide().collect();
    let absolute: Vec<u16> = match std::path::absolute(path) {
        Ok(absolute)
            if !spelled.starts_with(&[0x5c, 0x5c, 0x3f, 0x5c])
                && !spelled.starts_with(&[0x5c, 0x3f, 0x3f, 0x5c]) =>
        {
            absolute.as_os_str().encode_wide().collect()
        }
        _ => vec![],
    };
    if absolute.len() < LEGACY_MAX_PATH {
        return spelled.into_iter().chain(Some(0)).collect();
    }
    let (prefix, rest): (&str, &[u16]) = match absolute.as_slice() {
        [_, 0x3a, 0x5c, ..] => ("\\\\?\\", &absolute),
        [0x5c, 0x5c, 0x2e, 0x5c, rest @ ..] => ("\\\\?\\", rest),
        [0x5c, 0x5c, rest @ ..] => ("\\\\?\\UNC\\", rest),
        _ => ("", &absolute),
    };
    prefix
        .encode_utf16()
        .chain(rest.iter().copied())
        .chain(Some(0))
        .collect()
}

/// What a prepared temporary holds.
pub(crate) enum Source<'a> {
    /// The whole payload.
    Bytes(&'a [u8]),
    /// An open file copied in, refused once it passes the byte cap
    /// (`u64::MAX`: no cap). A capture source is read through its open
    /// handle, never reopened by path. The concrete `&File` (not `dyn Read`)
    /// keeps std's file-to-file copy fast path (`copy_file_range` on Linux):
    /// a few syscalls per copy instead of one read and one write per 8 KiB.
    File(&'a fs::File, u64),
}

/// The temporary's file mode at creation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The platform default (umask, inherited ACL).
    Default,
    /// Owner-only for a file that holds secrets (quick share's account token
    /// and share keys): mode 0600 at creation, not a later chmod, on the
    /// Unix-family targets, kept through fsync and rename; on Windows the
    /// per-user app-data ACL is inherited. An unknown target refuses rather
    /// than write a secret with default permissions.
    Private,
}

/// The one write-temp, fsync, rename recipe (`docs/storage-contract.md`), split at
/// its synced-temp barrier: [`PreparedWrite::prepare`] leaves a written and
/// fsynced same-directory temp, and the caller's base-revision check runs
/// between it and [`PreparedWrite::publish`]. The caller retains its base guard
/// and page lock through publication and sync. Cost per write: 1 temp file, 1
/// file fsync, 1 rename, plus 1 directory fsync in
/// [`PreparedWrite::publish_and_sync`] (the page host syncs the directory
/// itself, for its witness).
pub(crate) struct PreparedWrite {
    temporary: PathBuf,
    target: PathBuf,
    published: bool,
}

impl PreparedWrite {
    /// Create `temp_path(target, seq, tag)` with `mode`, write `source` into
    /// it and fsync it. On any failure the temp is gone and the target is
    /// untouched.
    pub(crate) fn prepare(
        target: &Path,
        source: Source<'_>,
        mode: Mode,
        tag: &str,
    ) -> io::Result<Self> {
        let options = temp_options(mode)?;
        let temporary = temp_path(target, TMP_SEQ.fetch_add(1, Ordering::Relaxed), tag);
        let mut file = options
            .open(&temporary)
            .map_err(at("create temporary file"))?;
        let prepared = Self {
            temporary,
            target: target.to_path_buf(),
            published: false,
        };
        let written = match source {
            Source::Bytes(bytes) => {
                file.write_all(bytes).map_err(at("write temporary file"))?;
                bytes.len() as u64
            }
            Source::File(input, max_bytes) => {
                let mut limited = io::Read::take(input, max_bytes.saturating_add(1));
                let copied =
                    io::copy(&mut limited, &mut file).map_err(at("write temporary file"))?;
                if copied > max_bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("capture exceeds {max_bytes} byte limit"),
                    ));
                }
                copied
            }
        };
        count_write(written);
        sync_file_handle(&file).map_err(at("fsync temporary file"))?;
        Ok(prepared)
    }

    /// Rename the temp onto the target: no-replace for a `create`, else
    /// replace. The caller owns the directory sync.
    pub(crate) fn publish(mut self, create: bool) -> io::Result<()> {
        if create {
            super::no_replace::move_file_noreplace(&self.temporary, &self.target)?;
        } else {
            rename_replace(&self.temporary, &self.target)
                .map_err(at("rename temporary file over target"))?;
        }
        self.published = true;
        Ok(())
    }

    /// [`Self::publish`], then fsync the target's directory.
    pub(crate) fn publish_and_sync(self, create: bool) -> io::Result<()> {
        let dir = self
            .target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        self.publish(create)?;
        sync_target_directory(&dir)
    }
}

impl Drop for PreparedWrite {
    fn drop(&mut self) {
        // Only this protocol's unpublished scratch file; never the target.
        if !self.published {
            let _ = fs::remove_file(&self.temporary);
        }
    }
}

/// The temp's open options: always a new file, owner-only for [`Mode::Private`].
fn temp_options(mode: Mode) -> io::Result<fs::OpenOptions> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    if mode == Mode::Private {
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios"
        ))]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios",
            target_os = "windows"
        )))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private files are unavailable on this target",
        ));
    }
    Ok(options)
}

#[cfg(test)]
thread_local! {
    pub(crate) static FAIL_FILE_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(crate) static FILE_SYNCS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    pub(crate) static TEMP_WRITES: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// The process-wide `test-faults` cost counters (the app crate, which also
/// compiles this file, has none: [`counting`] is a no-op there).
#[cfg(feature = "test-faults")]
mod counting {
    use std::cell::Cell;

    thread_local! {
        static UNCOUNTED: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn uncounted<R>(write: impl FnOnce() -> R) -> R {
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                UNCOUNTED.with(|uncounted| uncounted.set(self.0));
            }
        }
        let _reset = Reset(UNCOUNTED.with(|uncounted| uncounted.replace(true)));
        write()
    }

    pub(super) fn fsync() {
        if !UNCOUNTED.with(Cell::get) {
            crate::cost_counters::fsync();
        }
    }

    pub(super) fn wrote(bytes: u64) {
        if !UNCOUNTED.with(Cell::get) {
            crate::cost_counters::wrote(bytes as usize);
        }
    }
}

#[cfg(not(feature = "test-faults"))]
mod counting {
    pub(super) fn uncounted<R>(write: impl FnOnce() -> R) -> R {
        write()
    }
    pub(super) fn fsync() {}
    pub(super) fn wrote(_: u64) {}
}

/// Run `write` off the process-wide `cost_counters`: for app-data writes
/// that are no part of a measured graph operation and may run on their own
/// thread during one (the launch checkpoint), which would race every
/// measured window. The thread-local unit-test counters still see them.
pub(crate) fn uncounted<R>(write: impl FnOnce() -> R) -> R {
    counting::uncounted(write)
}

/// Count one fsync (file or directory) for the cost tests.
fn count_fsync() {
    counting::fsync();
}

/// Count one temp payload of `bytes` for the cost tests.
fn count_write(bytes: u64) {
    #[cfg(test)]
    TEMP_WRITES.with(|count| {
        let (files, written) = count.get();
        count.set((files + 1, written + bytes));
    });
    counting::wrote(bytes);
}

/// Fsync `dir`'s entries ([`super::directory_durability::sync_directory_entry`]),
/// counted like a file fsync.
pub(crate) fn sync_target_directory(dir: &Path) -> io::Result<()> {
    count_fsync();
    super::directory_durability::sync_directory_entry(dir)
}

/// Create-new publication of `bytes` at `path` (temp tag `.new`): fails with
/// `AlreadyExists` if a file appeared there after the caller's check.
pub(crate) fn atomic_write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    PreparedWrite::prepare(path, Source::Bytes(bytes), Mode::Default, ".new")?
        .publish_and_sync(true)
}

/// [`atomic_write_with_check`] without a check, owner-only ([`Mode::Private`];
/// temp tag `.private`). Only the app's `device_io` calls it.
#[allow(dead_code)]
pub(crate) fn atomic_write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    PreparedWrite::prepare(path, Source::Bytes(bytes), Mode::Private, ".private")?
        .publish_and_sync(false)
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
    sync_target_directory(dir)
}

/// File part of the existing durability recipe.
fn sync_file_bytes(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    let file = fs::OpenOptions::new().write(true).open(path);
    #[cfg(not(windows))]
    let file = fs::File::open(path);
    file.and_then(|file| sync_file_handle(&file))
        .map_err(at("fsync unchanged file"))
}

/// The one file fsync: every count of it (`FILE_SYNCS` for unit tests, the
/// `test-faults` cost counters) and its injected failure live here.
pub(crate) fn sync_file_handle(file: &fs::File) -> io::Result<()> {
    #[cfg(test)]
    FILE_SYNCS.with(|count| count.set(count.get() + 1));
    count_fsync();
    #[cfg(test)]
    if FAIL_FILE_SYNC.with(|fail| fail.replace(false)) {
        return Err(io::Error::other("injected file sync failure"));
    }
    file.sync_all()
}

/// A replacing write of `bytes` whose `check` (the caller's base-revision
/// guard) runs after the temp is synced and before the rename.
pub(crate) fn atomic_write_with_check(
    path: &Path,
    bytes: &[u8],
    check: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let prepared = PreparedWrite::prepare(path, Source::Bytes(bytes), Mode::Default, "")?;
    check()?;
    prepared.publish_and_sync(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REVIEW-2b F10, hosted Windows only: a path past MAX_PATH takes the
    /// write-through call on std's verbatim form and replaces the target.
    #[cfg(windows)]
    #[test]
    fn windows_replace_handles_long_paths() {
        let root = tempfile::tempdir().unwrap();
        let mut dir = root.path().to_path_buf();
        while dir.as_os_str().len() < 300 {
            dir.push("a-directory-name-that-makes-the-path-long");
        }
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dst) = (dir.join("src.md"), dir.join("dst.md"));
        std::fs::write(&src, b"new").unwrap();
        std::fs::write(&dst, b"old").unwrap();
        super::rename_replace(&src, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"new");
        assert!(!src.exists());
    }

    /// REVIEW-2b F10, hosted Windows only: a read-only destination gets
    /// exactly std's outcome (its ACCESS_DENIED -> FileRenameInfoEx retry).
    #[cfg(windows)]
    #[test]
    fn windows_replace_matches_std_on_a_read_only_destination() {
        let mut outcomes = vec![];
        for ours in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let (src, dst) = (root.path().join("src.md"), root.path().join("dst.md"));
            std::fs::write(&src, b"new").unwrap();
            std::fs::write(&dst, b"old").unwrap();
            let mut permissions = std::fs::metadata(&dst).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&dst, permissions).unwrap();
            let result = if ours {
                super::rename_replace(&src, &dst)
            } else {
                std::fs::rename(&src, &dst)
            };
            outcomes.push((result.is_ok(), std::fs::read(&dst).unwrap(), src.exists()));
        }
        assert_eq!(outcomes[0], outcomes[1]);
    }

    /// REVIEW-2b-r2 R2, hosted Windows only: the shared form keeps every
    /// spelling under the legacy limit exactly and extends only long ones.
    #[cfg(windows)]
    #[test]
    fn windows_native_path_keeps_short_spellings_and_extends_long_ones() {
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        for short in [
            r"C:\g\pages\a.md",
            r"pages\a.md",
            r"\\server\share\a.md",
            r"\\?\C:\g\a.md",
        ] {
            assert_eq!(native_path(Path::new(short)), wide(short), "{short}");
        }
        let long = format!(r"C:\{}\a.md", "d".repeat(300));
        assert_eq!(native_path(Path::new(&long)), wide(&format!(r"\\?\{long}")));
        let unc = format!(r"\\server\share\{}\a.md", "d".repeat(300));
        assert_eq!(
            native_path(Path::new(&unc)),
            wide(&format!(r"\\?\UNC\{}", &unc[2..]))
        );
    }

    /// REVIEW-2b-r2 R2, hosted Windows only: no-replace and replace moves on
    /// short absolute, short relative, long (>300) and, where the admin
    /// share is reachable, UNC paths.
    #[cfg(windows)]
    #[test]
    fn windows_moves_short_relative_long_and_unc_paths() {
        fn both(dir: &Path) {
            let (src, dst) = (dir.join("src.md"), dir.join("dst.md"));
            fs::write(&src, b"one").unwrap();
            super::super::no_replace::move_file_noreplace(&src, &dst).unwrap();
            fs::write(&src, b"two").unwrap();
            let refused = super::super::no_replace::move_file_noreplace(&src, &dst).unwrap_err();
            assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists, "{dir:?}");
            rename_replace(&src, &dst).unwrap();
            assert_eq!(fs::read(&dst).unwrap(), b"two", "{dir:?}");
            assert!(!src.exists());
            fs::remove_file(&dst).unwrap();
        }
        let root = tempfile::tempdir().unwrap();
        both(root.path());
        let here = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        both(Path::new(here.path().file_name().unwrap()));
        let mut long = root.path().to_path_buf();
        while long.as_os_str().len() < 300 {
            long.push("a-directory-name-that-makes-the-path-long");
        }
        fs::create_dir_all(&long).unwrap();
        both(&long);
        let absolute = root.path().to_string_lossy().into_owned();
        if let [drive, b':', ..] = absolute.as_bytes() {
            let unc = format!(r"\\localhost\{}$\{}", *drive as char, &absolute[3..]);
            if Path::new(&unc).is_dir() {
                both(Path::new(&unc));
            } else {
                eprintln!("UNC admin share unavailable: {unc}");
            }
        }
    }

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

    /// A quick-share secret is owner-only from creation: the temp is created
    /// 0600 even under umask 0, and no chmod follows (the mode is set only in
    /// `temp_options`, and nothing in this file calls `set_permissions`).
    #[cfg(unix)]
    #[test]
    fn a_private_write_is_created_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("secret.json");
        let options = temp_options(Mode::Private).unwrap();
        let old = unsafe { libc::umask(0) };
        let probe = options.open(root.path().join("probe"));
        let result = atomic_write_private(&path, b"{}");
        unsafe { libc::umask(old) };
        probe.unwrap();
        result.unwrap();
        for file in [root.path().join("probe"), path] {
            assert_eq!(
                fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600,
                "{file:?}"
            );
        }
        let source = include_str!("atomic_file.rs");
        let code = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        assert!(!code.contains("set_permissions"));
        assert_eq!(code.matches(".mode(0o600)").count(), 1);
    }

    /// Every tag in use and a long, shortened name: `parse_temp_name` gives
    /// the tag back exactly, and the target name the temp carries: the full
    /// name when it fits, else the cut stem with the extension kept.
    #[test]
    fn parse_temp_name_reads_back_every_temp_path() {
        let tags = ["", ".new", ".private", ".restore", ".capture"];
        let long = format!("{}.md", "漢".repeat(84));
        for name in ["A.md", "assets/x.png", "config.edn", long.as_str()] {
            let target = Path::new("/g/pages").join(name);
            let fname = target.file_name().unwrap().to_str().unwrap();
            for tag in tags {
                for seq in [0, 7, u64::MAX] {
                    let temp = temp_path(&target, seq, tag);
                    let temp = temp.file_name().unwrap().to_str().unwrap();
                    let (carried, parsed_tag) = parse_temp_name(temp).expect(temp);
                    assert_eq!(parsed_tag, tag, "{temp}");
                    if name != long {
                        assert_eq!(carried, fname, "{temp}");
                    } else {
                        let (stem, ext) = split_short_ext(fname);
                        let (cut, carried_ext) = split_short_ext(carried);
                        assert_eq!(carried_ext, ext, "{temp}");
                        assert!(stem.starts_with(cut) && !cut.is_empty(), "{temp}");
                    }
                }
            }
        }
        let tx = fixed_temp_name("tx", "1759000000000-1");
        assert_eq!(tx, ".tine-tx-1759000000000-1.tmp");
        assert!(is_fixed_temp_name(&tx));
        let stage = temp_path(&Path::new("/g/pages").join(&tx), 3, ".capture");
        let (carried, tag) = parse_temp_name(stage.file_name().unwrap().to_str().unwrap()).unwrap();
        assert_eq!((carried, tag), (tx.as_str(), ".capture"));
        for foreign in [
            "A.md",
            ".A.md.tmp",
            ".A.md.12.tmp",
            ".A.md.x.7.tmp",
            ".A.md.1.x.tmp",
            "A.md.1.2.tmp",
            ".1.2.tmp",
            ".A.md.1.2.new.part",
        ] {
            assert_eq!(parse_temp_name(foreign), None, "{foreign}");
        }
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

    /// Cases that need the rest of tine-store (`model`'s copy helpers, the
    /// cost counters, the crate's own sources): the app crate compiles this
    /// file too, without them.
    #[cfg(feature = "test-faults")]
    mod store {
        use super::*;

        type Helper<'a> = (&'static str, Box<dyn Fn(&Path) -> io::Result<()> + 'a>);

        /// Every write helper, in one place: the four copies this file and
        /// `model.rs` used to carry, and the replacing write.
        fn helpers(source: &Path) -> Vec<Helper<'_>> {
            vec![
                (
                    "atomic_write_new",
                    Box::new(|p| atomic_write_new(p, b"payload")),
                ),
                (
                    "atomic_write_private",
                    Box::new(|p| atomic_write_private(p, b"payload")),
                ),
                (
                    "atomic_write_with_check",
                    Box::new(|p| atomic_write_with_check(p, b"payload", || Ok(()))),
                ),
                (
                    "atomic_copy_new",
                    Box::new(move |p| crate::model::atomic_copy_new(source, p)),
                ),
                (
                    "atomic_copy_file_new",
                    Box::new(move |p| {
                        crate::model::atomic_copy_file_new(&mut fs::File::open(source)?, p, 64)
                    }),
                ),
            ]
        }

        fn entries(dir: &Path) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }

        /// One call of each helper: one temp payload, one file fsync and one
        /// directory fsync on the thread's counters, and at least those two on
        /// the process-wide cost counters (other tests' writes only add to them;
        /// the exact count is `tests/cb1_temp_recipe.rs`'s, under the lock). The
        /// copy helpers used to report 0 file fsyncs here.
        #[test]
        fn every_write_helper_syncs_its_file_and_directory_once() {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.bin");
            fs::write(&source, b"payload").unwrap();
            crate::model::checked_write_target(root.path(), root.path(), "fixture", None).unwrap();
            for (name, write) in helpers(&source) {
                let target = root.path().join(format!("{name}.bin"));
                FILE_SYNCS.with(|count| count.set(0));
                TEMP_WRITES.with(|count| count.set((0, 0)));
                let _ = crate::directory_durability::take_synced_directories();
                let before = crate::cost_counters::snapshot();
                write(&target).unwrap();
                let after = crate::cost_counters::snapshot();
                assert_eq!(FILE_SYNCS.with(std::cell::Cell::get), 1, "{name}");
                assert_eq!(TEMP_WRITES.with(std::cell::Cell::get), (1, 7), "{name}");
                assert_eq!(
                    crate::directory_durability::take_synced_directories(),
                    [root.path()],
                    "{name}"
                );
                assert!(after.fsyncs - before.fsyncs >= 2, "{name}");
                assert!(after.files_written > before.files_written, "{name}");
                assert_eq!(fs::read(&target).unwrap(), b"payload", "{name}");
            }
            let mut expected: Vec<String> = helpers(&source)
                .iter()
                .map(|(name, _)| format!("{name}.bin"))
                .chain(["source.bin".into()])
                .collect();
            expected.sort();
            assert_eq!(entries(root.path()), expected, "no temp is left");
        }

        /// A failed file fsync under each helper fails the write, leaves no temp
        /// and leaves the target as it was (absent, or its old bytes).
        #[test]
        fn a_failed_file_sync_leaves_no_temp_and_an_unchanged_target() {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.bin");
            fs::write(&source, b"payload").unwrap();
            crate::model::checked_write_target(root.path(), root.path(), "fixture", None).unwrap();
            for (name, write) in helpers(&source) {
                let target = root.path().join(format!("{name}.bin"));
                let replaces = matches!(name, "atomic_write_private" | "atomic_write_with_check");
                if replaces {
                    fs::write(&target, b"old").unwrap();
                }
                FAIL_FILE_SYNC.with(|fail| fail.set(true));
                let error = write(&target).unwrap_err();
                assert_eq!(error.to_string(), "injected file sync failure", "{name}");
                assert_eq!(
                    fs::read(&target).ok(),
                    replaces.then(|| b"old".to_vec()),
                    "{name}"
                );
                let mut expected = vec!["source.bin".to_string()];
                if replaces {
                    expected.push(format!("{name}.bin"));
                }
                expected.sort();
                assert_eq!(entries(root.path()), expected, "{name}: a temp was left");
                let _ = fs::remove_file(&target);
            }
        }

        /// A capture past its cap fails, leaves no temp and creates no target.
        #[test]
        fn an_over_cap_capture_leaves_nothing() {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.bin");
            fs::write(&source, b"payload").unwrap();
            crate::model::checked_write_target(root.path(), root.path(), "fixture", None).unwrap();
            let target = root.path().join("capture.bin");
            let error = crate::model::atomic_copy_file_new(
                &mut fs::File::open(&source).unwrap(),
                &target,
                3,
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(entries(root.path()), ["source.bin"]);
        }

        /// This thread's read and write syscall counts (`/proc/thread-self/io`),
        /// so a parallel test's I/O cannot leak into the window. Linux only.
        fn thread_syscalls() -> (u64, u64) {
            let io = fs::read_to_string("/proc/thread-self/io").unwrap();
            let field = |name: &str| -> u64 {
                io.lines()
                    .find_map(|line| line.strip_prefix(name))
                    .and_then(|rest| rest.trim().parse().ok())
                    .unwrap()
            };
            (field("syscr:"), field("syscw:"))
        }

        /// A file-backed copy keeps std's file-to-file fast path: an 8 MiB
        /// import costs a handful of read and write syscalls, not one per
        /// 8 KiB buffer (2,048 here), in the unlimited, the at-cap and the
        /// over-cap forms; the cap still leaves nothing past it. Linux only (a
        /// runtime check, so it adds no platform arm to I-16's census).
        #[test]
        fn a_streamed_copy_keeps_the_file_to_file_fast_path() {
            if !cfg!(target_os = "linux") {
                return;
            }
            const LEN: usize = 8 << 20;
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.bin");
            let payload: Vec<u8> = (0..LEN).map(|i| (i % 251) as u8).collect();
            fs::write(&source, &payload).unwrap();
            crate::model::checked_write_target(root.path(), root.path(), "fixture", None).unwrap();
            let len = LEN as u64;
            let copies: [(&str, Box<dyn Fn(&Path) -> io::Result<()>>); 3] = [
                (
                    "atomic_copy_new",
                    Box::new(|p| crate::model::atomic_copy_new(&source, p)),
                ),
                (
                    "atomic_copy_file_new at cap",
                    Box::new(|p| {
                        crate::model::atomic_copy_file_new(&mut fs::File::open(&source)?, p, len)
                    }),
                ),
                (
                    "atomic_copy_file_new over cap",
                    Box::new(|p| {
                        crate::model::atomic_copy_file_new(
                            &mut fs::File::open(&source)?,
                            p,
                            len - 1,
                        )
                    }),
                ),
            ];
            for (index, (name, copy)) in copies.iter().enumerate() {
                let target = root.path().join(format!("copy-{index}.bin"));
                let (reads, writes) = thread_syscalls();
                let started = std::time::Instant::now();
                let result = copy(&target);
                let elapsed = started.elapsed();
                let (reads_after, writes_after) = thread_syscalls();
                let syscalls = (reads_after - reads, writes_after - writes);
                eprintln!("{name}: {syscalls:?} read/write syscalls, {elapsed:?}");
                assert!(
                    syscalls.0 + syscalls.1 < 64,
                    "{name}: {syscalls:?} read/write syscalls for {LEN} bytes"
                );
                if name.ends_with("over cap") {
                    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
                    assert!(!target.exists(), "{name}: nothing past the cap");
                } else {
                    result.unwrap();
                    assert!(fs::read(&target).unwrap() == payload, "{name}: bytes");
                }
            }
            assert_eq!(
                entries(root.path()),
                ["copy-0.bin", "copy-1.bin", "source.bin"]
            );
        }

        /// The recipe stays single: outside restore's capability-handle
        /// copy and the publisher's staging protocol, `create_new(true)` appears
        /// once in tine-store's shipped code (`temp_options`), and a file's
        /// `sync_all()` only inside `sync_file_handle` (`directory_durability`
        /// syncs directory handles). Not caught: a recipe in `src-tauri` written
        /// with plain `std::fs`, or a new fixed temp name.
        #[test]
        fn the_write_recipe_has_one_create_and_one_file_sync() {
            fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
                for entry in fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        walk(&path, out);
                    } else if path.extension().is_some_and(|ext| ext == "rs") {
                        out.push(path);
                    }
                }
            }
            let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
            let mut files = vec![];
            walk(&src, &mut files);
            let (mut creates, mut syncs) = (vec![], vec![]);
            for file in files {
                let relative = file
                    .strip_prefix(&src)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let stem = file.file_stem().unwrap().to_string_lossy();
                if stem.contains("test") || matches!(relative.as_str(), "restore.rs" | "publish.rs")
                {
                    continue;
                }
                let source = fs::read_to_string(&file).unwrap();
                let shipped = source
                    .find("#[cfg(test)]\nmod tests")
                    .map_or(source.as_str(), |end| &source[..end]);
                for line in shipped
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                {
                    if line.contains("create_new(true)") {
                        creates.push(relative.clone());
                    }
                    if line.contains("sync_all()") {
                        syncs.push(relative.clone());
                    }
                }
            }
            assert_eq!(creates, ["atomic_file.rs"]);
            syncs.sort();
            assert_eq!(syncs, ["atomic_file.rs", "directory_durability.rs"]);
            let atomic = include_str!("atomic_file.rs");
            let body = &atomic[atomic.find("pub(crate) fn sync_file_handle").unwrap()..];
            assert!(body[..body.find("\n}\n").unwrap()].contains("file.sync_all()"));
        }
    }
}
