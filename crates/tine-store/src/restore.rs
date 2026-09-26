//! Replace the graph's live page, journal, and asset-sidecar set from backup files.

use crate::store::{Area, FileId, GraphRev, Store};
use crate::IoError;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const ASSET_RECOVERY: &str = ".tine-restore-recovery";
static RECOVERY_SEQ: AtomicU64 = AtomicU64::new(0);
static COPY_SEQ: AtomicU64 = AtomicU64::new(0);

/// One caller-supplied snapshot file. `rel` is relative to `area`; `source` is
/// an already-open file. Restore checks regular-file metadata and `len`, but
/// does not verify file content against a backup checksum.
pub struct RestoreFile {
    /// Destination graph area.
    pub area: Area,
    /// Destination name relative to `area`.
    pub rel: String,
    /// Open source file copied from its beginning.
    pub source: File,
    /// Expected source length, checked before and after copying.
    pub len: u64,
}

/// Completed work and recovery locations, including after a partial failure.
#[derive(Debug)]
pub struct RestoreReport {
    /// Number of input files copied into the graph.
    pub restored: u64,
    /// Same-filesystem recovery directories holding retired files.
    pub recovery: Vec<PathBuf>,
    /// Live targets left in place when no-replace copying found a concurrent
    /// target. Its bytes need not differ from the restore baseline.
    pub kept_external: Vec<FileId>,
    /// Generation published for a changed disk state, or the current generation
    /// if restore made no change. During a failed initial load this is the
    /// unchanged current revision even if restore wrote files: no view covers
    /// those writes until recovery's first view does.
    pub graph_rev: GraphRev,
}

/// A restore stopped at `phase`; `done` describes work already completed.
#[derive(Debug)]
pub struct RestoreFailed {
    /// Human-readable phase description. This is not a stable enum or a value
    /// suitable for programmatic branching.
    pub phase: String,
    /// Error that stopped the restore.
    pub cause: IoError,
    /// Work completed before the failure; recovery locations remain available.
    pub done: RestoreReport,
}

struct Recovery {
    root_path: PathBuf,
    root: Dir,
    dir: Dir,
    path: PathBuf,
}

fn fail(phase: &str, cause: io::Error, done: RestoreReport) -> RestoreFailed {
    RestoreFailed {
        phase: phase.into(),
        cause: cause.into(),
        done,
    }
}

impl Store {
    /// Restore page and journal text, asset `.edn` sidecars, and config from
    /// open files whose supplied lengths are checked before copying. The
    /// caller is responsible for stronger source verification. The input is
    /// the complete desired set in those
    /// areas: every unlisted live page, journal, and asset sidecar is retired
    /// into same-filesystem recovery roots. Graph files are retired under
    /// `logseq/.tine-trash/<restore-id>`; asset sidecars under
    /// `assets/.tine-restore-recovery/<restore-id>`. The returned `recovery`
    /// paths locate them; the store has no restore-import or cleanup call.
    /// Replaced files are retired too. An unlisted `config.edn` and
    /// `custom.css` stay live. Only `config.edn` is accepted in the Meta area;
    /// Trash targets and non-`.edn` assets are refused. Any `.edn` file under
    /// assets counts as a sidecar, regardless of a matching PDF.
    /// Other asset files are left in place. The method then copies new files
    /// without replacing a concurrent winner. It blocks saves and transactions
    /// for the full operation. Cost includes all input bytes, all live page,
    /// journal, and sidecar bytes hashed for baseline and publication, and an
    /// asset-tree walk, even for a small input. A changed restore publishes one
    /// `Origin::Own` revision for the final disk state, including `Removed`
    /// tuples for retired live files and `config_changed` when config changed.
    /// The config is reloaded before the resulting view is published. A changed
    /// partial result on failure publishes the final disk state after a
    /// successful initial load. After a failed initial load, writes remain
    /// guarded but publication waits for successful `scan_refresh()` recovery.
    /// An in-flight save holding the writer lock
    /// finishes before this restore; a later save checks against restored
    /// bytes. Check `recovery` and
    /// `kept_external` when reconciling disk state. Existing `WholeGraph` views
    /// remain captured snapshots until the final publication; direct `page()`
    /// and `scan_area()` calls can observe intermediate files because they read
    /// disk without the restore writer lock. The watcher waits for that lock.
    /// A crash can leave a partial restore with whole individual files and
    /// recovery directories; there is no store import or cleanup call.
    /// An editor must separately
    /// preserve its unsaved buffer and compare its base revision before saving.
    pub fn restore(&self, mut files: Vec<RestoreFile>) -> Result<RestoreReport, RestoreFailed> {
        let _writer = self.writer.lock().unwrap();
        let mut done = RestoreReport {
            restored: 0,
            recovery: Vec::new(),
            kept_external: Vec::new(),
            graph_rev: self.changes.rev(),
        };
        if self.is_closed() {
            return Err(fail(
                "restore",
                io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
                done,
            ));
        }
        for file in &files {
            let allowed = match file.area {
                Area::Pages | Area::Journals => is_graph_text(Path::new(&file.rel)),
                Area::Assets => {
                    is_sidecar(Path::new(&file.rel))
                        && !file.rel.split('/').any(|part| part == ASSET_RECOVERY)
                }
                Area::Meta => file.rel == "config.edn",
                Area::Trash => false,
            };
            if !allowed || self.file_id(file.area, &file.rel).is_err() {
                return Err(fail(
                    "restore",
                    io::Error::new(io::ErrorKind::InvalidInput, "unsafe restore file"),
                    done,
                ));
            }
            match file.source.metadata() {
                Ok(meta) if meta.is_file() && meta.len() == file.len => {}
                Ok(_) => {
                    return Err(fail(
                        "restore",
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "verified restore source changed",
                        ),
                        done,
                    ))
                }
                Err(error) => return Err(fail("restore", error, done)),
            }
        }
        let baseline = self.watch.restore_baseline();
        let root_path = self.graph.root.clone();
        let assets_path = self.graph.assets_path();
        for (label, path) in [
            (
                "journals",
                root_path.join(&self.graph.current_config().journals_dir),
            ),
            (
                "pages",
                root_path.join(&self.graph.current_config().pages_dir),
            ),
            ("config", root_path.join("logseq/config.edn")),
        ] {
            if let Err(error) = ensure_target_within_root(&root_path, &path) {
                return Err(fail(&format!("unsafe live {label} path"), error, done));
            }
        }
        if std::fs::canonicalize(root_path.join("assets"))
            .ok()
            .as_ref()
            != Some(&assets_path)
        {
            return Err(fail(
                "unsafe live assets path",
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "external assets directory changed",
                ),
                done,
            ));
        }
        let recovery_id = format!(
            "{}-pre-restore-extras-{}-{}",
            backup_stamp(),
            std::process::id(),
            RECOVERY_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let graph = match reserve(&root_path, Path::new("logseq/.tine-trash"), &recovery_id) {
            Ok(value) => value,
            Err(error) => return Err(fail("couldn't create restore recovery area", error, done)),
        };
        done.recovery.push(graph.path.clone());
        let assets = match reserve(&assets_path, Path::new(ASSET_RECOVERY), &recovery_id) {
            Ok(value) => value,
            Err(error) => {
                return Err(fail(
                    "couldn't create asset restore recovery area",
                    error,
                    done,
                ))
            }
        };
        done.recovery.push(assets.path.clone());
        #[cfg(feature = "test-faults")]
        if let Err(error) = pause_after_binding_for_test(&root_path) {
            return Err(fail("restore", error, done));
        }
        let mut changed = false;

        // The old protocol completes one area before starting the next one.
        for (area, phase, live_prefix, recovery_prefix) in [
            (
                Area::Journals,
                "restore journals failed",
                self.graph.current_config().journals_dir.as_str(),
                "journals",
            ),
            (
                Area::Pages,
                "restore pages failed",
                self.graph.current_config().pages_dir.as_str(),
                "pages",
            ),
            (Area::Assets, "restore asset sidecars failed", "", ""),
        ] {
            let bound = if area == Area::Assets {
                &assets
            } else {
                &graph
            };
            let mut restored = HashSet::new();
            for file in files.iter_mut().filter(|file| file.area == area) {
                let live_rel = if live_prefix.is_empty() {
                    PathBuf::from(&file.rel)
                } else {
                    Path::new(live_prefix).join(&file.rel)
                };
                let recover_rel = if recovery_prefix.is_empty() {
                    PathBuf::from(&file.rel)
                } else {
                    Path::new(recovery_prefix).join(&file.rel)
                };
                let mut copying = false;
                let result: io::Result<()> = (|| {
                    if move_if_present(bound, &live_rel, &recover_rel)? {
                        changed = true;
                    }
                    copying = true;
                    copy_new(bound, &live_rel, &mut file.source, file.len)?;
                    changed = true;
                    Ok(())
                })();
                if let Err(error) = result {
                    if copying && error.kind() == io::ErrorKind::AlreadyExists {
                        if let Ok(id) = self.file_id(area, &file.rel) {
                            done.kept_external.push(id);
                        }
                    }
                    if changed {
                        self.graph.invalidate_cache();
                        done.graph_rev = self.watch.publish_restore(&baseline);
                    }
                    return Err(fail(phase, error, done));
                }
                restored.insert(PathBuf::from(&file.rel));
                done.restored += 1;
            }
            let live_dir = if live_prefix.is_empty() {
                Path::new("")
            } else {
                Path::new(live_prefix)
            };
            if let Err(error) = retire_extras(
                bound,
                live_dir,
                Path::new(recovery_prefix),
                Path::new(""),
                &restored,
                area,
                &mut changed,
            ) {
                if changed {
                    self.graph.invalidate_cache();
                    done.graph_rev = self.watch.publish_restore(&baseline);
                }
                return Err(fail(phase, error, done));
            }
        }
        if let Some(file) = files.iter_mut().find(|file| file.area == Area::Meta) {
            let live = Path::new("logseq/config.edn");
            if let Err(error) = real_parent(&graph.root, Path::new("logseq"), true) {
                if changed {
                    self.graph.invalidate_cache();
                    done.graph_rev = self.watch.publish_restore(&baseline);
                }
                return Err(fail("couldn't prepare live config directory", error, done));
            }
            match move_if_present(&graph, live, live) {
                Ok(true) => changed = true,
                Ok(false) => {}
                Err(error) => {
                    if changed {
                        self.graph.invalidate_cache();
                        done.graph_rev = self.watch.publish_restore(&baseline);
                    }
                    return Err(fail("recover current config failed", error, done));
                }
            }
            if let Err(error) = copy_new(&graph, live, &mut file.source, file.len) {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    if let Ok(id) = self.file_id(Area::Meta, "config.edn") {
                        done.kept_external.push(id);
                    }
                }
                if changed {
                    self.graph.invalidate_cache();
                    done.graph_rev = self.watch.publish_restore(&baseline);
                }
                return Err(fail("restore config failed", error, done));
            }
            changed = true;
            done.restored += 1;
        }
        if changed {
            self.graph.invalidate_cache();
        }
        done.graph_rev = self.watch.publish_restore(&baseline);
        Ok(done)
    }
}

fn is_graph_text(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|x| x.to_str()),
        Some("md" | "org")
    )
}

fn backup_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
}
fn is_sidecar(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()) == Some("edn")
}

fn ensure_target_within_root(root: &Path, target: &Path) -> io::Result<()> {
    let canonical_root = std::fs::canonicalize(root)?;
    let mut existing = target;
    while std::fs::symlink_metadata(existing).is_err() {
        existing = existing.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "target has no existing ancestor",
            )
        })?;
    }
    let canonical_existing = std::fs::canonicalize(existing)?;
    let expected = existing
        .strip_prefix(root)
        .map(|rel| canonical_root.join(rel))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target is outside graph root"))?;
    if canonical_existing == expected {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target escapes graph root",
        ))
    }
}

fn reserve(root_path: &Path, parent_rel: &Path, id: &str) -> io::Result<Recovery> {
    let root = Dir::open_ambient_dir(root_path, ambient_authority())?;
    let parent = real_parent(&root, parent_rel, true)?;
    parent.create_dir(id)?;
    let dir = parent.open_dir(id)?;
    Ok(Recovery {
        root_path: root_path.to_path_buf(),
        root,
        dir,
        path: root_path.join(parent_rel).join(id),
    })
}

fn real_parent(root: &Dir, rel: &Path, create: bool) -> io::Result<Dir> {
    let mut current = root.try_clone()?;
    let path_kind = if create {
        "restore recovery"
    } else {
        "live restore"
    };
    for component in rel.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{path_kind} path is not relative"),
            ));
        };
        if create {
            match current.create_dir(name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        let meta = current.symlink_metadata(name)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{path_kind} path contains a non-directory entry"),
            ));
        }
        current = current.open_dir(name)?;
    }
    Ok(current)
}

fn move_if_present(recovery: &Recovery, live: &Path, recover: &Path) -> io::Result<bool> {
    let name = live
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing live file name"))?;
    let live_parent = match real_parent(
        &recovery.root,
        live.parent().unwrap_or(Path::new("")),
        false,
    ) {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    match live_parent.symlink_metadata(name) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Ok(meta) if meta.file_type().is_symlink() => {
            ensure_target_within_root(&recovery.root_path, &recovery.root_path.join(live))?;
        }
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "live restore target is not a regular file",
            ))
        }
        Err(error) => return Err(error),
    }
    let recovery_name = recover
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing recovery file name"))?;
    let recovery_parent = real_parent(
        &recovery.dir,
        recover.parent().unwrap_or(Path::new("")),
        true,
    )?;
    match recovery_parent.symlink_metadata(recovery_name) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "restore recovery destination already exists",
            ))
        }
        Err(error) => return Err(error),
    }
    match rename_noreplace(
        &live_parent,
        Path::new(name),
        &recovery_parent,
        recovery_name,
    ) {
        Ok(()) => Ok(true),
        Err(rename_error) => {
            let mut source = live_parent.open(name)?.into_std();
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            let mut copy = recovery_parent
                .open_with(recovery_name, &options)?
                .into_std();
            io::copy(&mut source, &mut copy)?;
            copy.sync_all()?;
            Err(io::Error::new(rename_error.kind(), format!(
                "live file copied to recovery but could not be atomically detached: {rename_error}")))
        }
    }
}

fn copy_new(recovery: &Recovery, live: &Path, source: &mut File, len: u64) -> io::Result<()> {
    let name = live
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing live file name"))?;
    let parent = real_parent(&recovery.root, live.parent().unwrap_or(Path::new("")), true)?;
    let temp = format!(
        ".tine-restore-{}-{}.tmp",
        std::process::id(),
        COPY_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut output = parent.open_with(&temp, &options)?.into_std();
        source.seek(SeekFrom::Start(0))?;
        let copied = io::copy(source, &mut output)?;
        if copied != len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "verified restore source changed",
            ));
        }
        output.sync_all()?;
        drop(output);
        publish_temp(&parent, Path::new(&temp), name)?;
        if let Ok(sync) = parent.try_clone() {
            let _ = sync.into_std_file().sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = parent.remove_file(&temp);
    }
    result
}

fn retire_extras(
    recovery: &Recovery,
    live_dir: &Path,
    recovery_prefix: &Path,
    rel: &Path,
    restored: &HashSet<PathBuf>,
    area: Area,
    changed: &mut bool,
) -> io::Result<()> {
    let current = match real_parent(&recovery.root, &live_dir.join(rel), false) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in current.read_dir(".")? {
        let entry = entry?;
        let name = entry.file_name();
        let child = rel.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if area == Area::Assets {
                if name == ASSET_RECOVERY {
                    continue;
                }
            } else if name.to_str().is_none_or(|s| s.starts_with('.')) {
                continue;
            }
            retire_extras(
                recovery,
                live_dir,
                recovery_prefix,
                &child,
                restored,
                area,
                changed,
            )?;
        } else if ((area == Area::Assets && kind.is_file() && is_sidecar(&child))
            || (area != Area::Assets
                && (kind.is_file() || kind.is_symlink())
                && is_graph_text(&child)))
            && !restored.contains(&child)
        {
            let live = live_dir.join(&child);
            let recover = recovery_prefix.join(&child);
            if move_if_present(recovery, &live, &recover)? {
                *changed = true;
            }
        }
    }
    Ok(())
}

fn rename_noreplace(
    from_dir: &Dir,
    from: &Path,
    to_dir: &Dir,
    to: &std::ffi::OsStr,
) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
        let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(to.as_bytes())?;
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                from_dir.as_raw_fd(),
                from.as_ptr(),
                to_dir.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_NOREPLACE as libc::c_uint,
            )
        };
        return (result == 0)
            .then_some(())
            .ok_or_else(io::Error::last_os_error);
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
        let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(to.as_bytes())?;
        let result = unsafe {
            libc::renameatx_np(
                from_dir.as_raw_fd(),
                from.as_ptr(),
                to_dir.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_EXCL as libc::c_uint,
            )
        };
        return (result == 0)
            .then_some(())
            .ok_or_else(io::Error::last_os_error);
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    {
        from_dir.rename(from, to_dir, Path::new(to))
    }
}

fn publish_temp(parent: &Dir, temp: &Path, name: &std::ffi::OsStr) -> io::Result<()> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        rename_noreplace(parent, temp, parent, name)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    {
        parent.hard_link(temp, parent, Path::new(name))?;
        parent.remove_file(temp)
    }
}

#[cfg(feature = "test-faults")]
fn pause_after_binding_for_test(root: &Path) -> io::Result<()> {
    let request = root.join(".tine-restore-test-pause");
    if !request.exists() {
        return Ok(());
    }
    std::fs::write(root.join(".tine-restore-test-paused"), b"ready")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !root.join(".tine-restore-test-resume").exists() {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "restore test pause timed out",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
