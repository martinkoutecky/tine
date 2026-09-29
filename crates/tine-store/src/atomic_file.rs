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
/// page temp; uniqueness comes from pid + seq alone.
pub(crate) fn temp_path(path: &Path, seq: u64, tag: &str) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let suffix = format!(".{}.{seq}{tag}.tmp", std::process::id());
    let full = format!(".{fname}{suffix}");
    let limit = fname.len().max(TEMP_NAME_FLOOR).min(NAME_MAX_BYTES);
    if full.len() <= limit {
        return dir.join(full);
    }
    let (stem, ext) = match fname.rfind('.') {
        Some(dot) if dot > 0 && fname.len() - dot <= 10 => fname.split_at(dot),
        _ => (fname, ""),
    };
    let room = limit.saturating_sub(1 + ext.len() + suffix.len());
    let mut cut = room.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    dir.join(format!(".{}{ext}{suffix}", &stem[..cut]))
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

pub(crate) fn atomic_write_with_check(
    path: &Path,
    bytes: &[u8],
    check: impl FnOnce() -> io::Result<()>,
    on_write: impl FnOnce(),
    on_file_sync: impl FnOnce(),
    on_dir_sync: impl FnOnce(),
) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = temp_path(path, WRITE_TMP_SEQ.fetch_add(1, Ordering::Relaxed), "");
    let res = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(at("create temporary file"))?;
        file.write_all(bytes).map_err(at("write temporary file"))?;
        on_write();
        file.sync_all().map_err(at("fsync temporary file"))?;
        on_file_sync();
        drop(file);
        check()?;
        fs::rename(&tmp, path).map_err(at("rename temporary file over target"))
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    } else {
        super::directory_durability::sync_directory_entry(dir)?;
        on_dir_sync();
    }
    res
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
        let short = temp_path(Path::new("/g/pages/A.md"), 7, "");
        assert_eq!(
            short.file_name().unwrap().to_str().unwrap(),
            format!(".A.md.{}.7.tmp", std::process::id())
        );
    }
}
