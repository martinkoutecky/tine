//! Store-owned static publication staging, retirement and identity verification.

use crate::model::Graph;
use crate::store::Store;
use crate::transaction::IoError;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
#[cfg(not(target_os = "windows"))]
use same_file::Handle as FileIdentity;
#[cfg(test)]
use std::cell::RefCell;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

// `same_file::Handle` keeps its Windows handle open without FILE_SHARE_DELETE,
// which makes MoveFileW reject the final stage rename. Keep a separately-opened
// identity handle that does share deletion instead. We first compare it against
// the bound capability while both are open, so an ambient path swap cannot make
// the identity refer to a different directory; the live handle then prevents
// file-ID reuse through the move and supports ReFS's full 128-bit identities.
#[cfg(target_os = "windows")]
#[derive(Debug)]
struct FileIdentity {
    _file: fs::File,
    volume: u64,
    id: [u8; 16],
}

#[cfg(target_os = "windows")]
impl PartialEq for FileIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.volume == other.volume && self.id == other.id
    }
}

#[cfg(target_os = "windows")]
impl Eq for FileIdentity {}

#[cfg(target_os = "windows")]
fn identity_from_file(file: fs::File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let mut information = FILE_ID_INFO::default();
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileIdentity {
        _file: file,
        volume: information.VolumeSerialNumber,
        id: information.FileId.Identifier,
    })
}

#[cfg(not(target_os = "windows"))]
fn identity_from_file(file: fs::File) -> io::Result<FileIdentity> {
    FileIdentity::from_file(file)
}

#[cfg(target_os = "windows")]
fn identity_from_path(path: &Path) -> io::Result<FileIdentity> {
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    identity_from_file(unsafe { fs::File::from_raw_handle(handle) })
}

#[cfg(not(target_os = "windows"))]
fn identity_from_path(path: &Path) -> io::Result<FileIdentity> {
    FileIdentity::from_path(path)
}

struct PublishStage {
    path: PathBuf,
    root: Dir,
    dir: Dir,
    identity: FileIdentity,
}

/// A staged publish file writer. Each call writes and fsyncs one new file.
pub struct SiteWriter {
    stage: PublishStage,
    files: u64,
}

impl SiteWriter {
    /// Write an area-relative file under the reserved stage. Cost O(bytes).
    pub fn write(&mut self, rel: &str, bytes: &[u8]) -> Result<(), IoError> {
        write_publish_stage_file(&self.stage, rel, bytes).map_err(IoError::from)?;
        self.files += 1;
        Ok(())
    }
}

/// A failed publish keeps any retired previous site in recovery.
#[derive(Debug)]
pub struct PublishFailed {
    pub cause: IoError,
    pub previous_kept: Option<PathBuf>,
}

/// The published site path is for handing to the OS or showing to the user.
#[derive(Debug)]
pub struct PublishReceipt {
    pub site: PathBuf,
    pub files: u64,
    pub previous_kept: Option<PathBuf>,
}

impl Store {
    /// Stage each emitted file with fsync, retire the previous site, move the
    /// stage by no-replace, then verify its identity. Holds the writer mutex.
    /// A late winner stays live; the previous site stays in recovery. Publishes
    /// no graph generation. Cost O(emitted bytes).
    pub fn publish_site(
        &self,
        emit: &mut dyn FnMut(&mut SiteWriter) -> Result<(), IoError>,
    ) -> Result<PublishReceipt, PublishFailed> {
        let _writer = self.writer.lock().unwrap();
        if self.is_closed() {
            return Err(PublishFailed {
                cause: io::Error::new(io::ErrorKind::BrokenPipe, "store closed").into(),
                previous_kept: None,
            });
        }
        let out = self.graph.root.join("publish");
        let setup = (|| {
            self.graph.ensure_write_target(&out)?;
            reserve_publish_stage(&self.graph)
        })();
        let stage = setup.map_err(|cause| PublishFailed {
            cause: cause.into(),
            previous_kept: None,
        })?;
        let mut writer = SiteWriter { stage, files: 0 };
        emit(&mut writer).map_err(|cause| PublishFailed {
            cause,
            previous_kept: None,
        })?;
        let files = writer.files;
        commit_publish_stage_report(&self.graph, writer.stage, &out).map_err(
            |(cause, previous_kept)| PublishFailed {
                cause: cause.into(),
                previous_kept,
            },
        )?;
        Ok(PublishReceipt {
            site: out,
            files,
            previous_kept: None,
        })
    }
}

struct PublishRecovery {
    path: PathBuf,
    dir: Dir,
}

#[cfg(target_os = "windows")]
fn dir_identity(dir: &Dir, path: &Path) -> io::Result<FileIdentity> {
    let capability = identity_from_file(dir.try_clone()?.into_std_file())?;
    let share_delete = identity_from_path(path)?;
    if capability != share_delete {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "static-publish staging path changed while binding its identity",
        ));
    }
    Ok(share_delete)
}

#[cfg(not(target_os = "windows"))]
fn dir_identity(dir: &Dir, _path: &Path) -> io::Result<FileIdentity> {
    identity_from_file(dir.try_clone()?.into_std_file())
}

fn reserve_publish_stage(graph: &Graph) -> io::Result<PublishStage> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = Dir::open_ambient_dir(&graph.root, ambient_authority())?;
    for _ in 0..128 {
        let name = format!(
            ".tine-publish-stage-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let path = graph.root.join(&name);
        graph.ensure_write_target(&path)?;
        match root.create_dir(&name) {
            Ok(()) => {
                let dir = root.open_dir(&name)?;
                let identity = dir_identity(&dir, &path)?;
                return Ok(PublishStage {
                    path,
                    root,
                    dir,
                    identity,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique static-publish staging directory",
    ))
}

fn write_publish_stage_file(stage: &PublishStage, name: &str, bytes: &[u8]) -> io::Result<()> {
    let relative = Path::new(name);
    if name.is_empty()
        || name.contains('\\')
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || relative.is_absolute()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "static-publish output name must be one file",
        ));
    }
    publish_stage_write_race_hook(stage)?;
    // All generation is relative to the directory handle reserved above. A
    // rename plus symlink/junction replacement of the ambient stage pathname
    // therefore cannot redirect an open or truncate outside the graph.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    if let Some(parent) = relative
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        stage.dir.create_dir_all(parent)?;
    }
    let mut file = stage.dir.open_with(relative, &options)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
thread_local! {
    static PUBLISH_STAGE_WRITE_SWAP: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static PUBLISH_RECOVERY_SWAP: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn replace_bound_dir_path(path: &Path, outside: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let displaced = path.with_file_name(format!(
            "{}.displaced",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("bound")
        ));
        fs::rename(path, &displaced)?;
        symlink(outside, path)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, outside);
        Ok(())
    }
}

#[cfg(test)]
fn publish_stage_write_race_hook(stage: &PublishStage) -> io::Result<()> {
    PUBLISH_STAGE_WRITE_SWAP.with(|outside| match outside.borrow_mut().take() {
        Some(outside) => replace_bound_dir_path(&stage.path, &outside),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn publish_stage_write_race_hook(_stage: &PublishStage) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn publish_recovery_race_hook(recovery: &PublishRecovery) -> io::Result<()> {
    PUBLISH_RECOVERY_SWAP.with(|outside| match outside.borrow_mut().take() {
        Some(outside) => replace_bound_dir_path(&recovery.path, &outside),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn publish_recovery_race_hook(_recovery: &PublishRecovery) -> io::Result<()> {
    Ok(())
}

fn reserve_publish_recovery(graph: &Graph, root: &Dir) -> io::Result<PublishRecovery> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let recovery_rel = Path::new("logseq").join(".tine-trash").join("conflicts");
    let recovery = graph.root.join(&recovery_rel);
    graph.ensure_write_target(&recovery)?;
    root.create_dir_all(&recovery_rel)?;
    let recovery_root = root.open_dir(&recovery_rel)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    for _ in 0..128 {
        let name = format!(
            "{stamp}-{}__previous-publish",
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        match recovery_root.create_dir(&name) {
            Ok(()) => {
                return Ok(PublishRecovery {
                    path: recovery.join(&name),
                    dir: recovery_root.open_dir(&name)?,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve static-publish recovery directory",
    ))
}

fn commit_publish_stage_report(
    graph: &Graph,
    stage: PublishStage,
    out: &Path,
) -> Result<(), (io::Error, Option<PathBuf>)> {
    graph
        .ensure_write_target(out)
        .map_err(|error| (error, None))?;
    // cap-std may represent a directory capability with an O_PATH descriptor on
    // Linux, which cannot itself be fsynced. Every generated file is fsynced;
    // directory durability remains best-effort, matching the other atomic paths.
    let _ = stage
        .dir
        .try_clone()
        .map_err(|error| (error, None))?
        .into_std_file()
        .sync_all();
    let PublishStage {
        path,
        root,
        dir,
        identity,
    } = stage;
    // Windows refuses to rename a directory while this capability is open.
    // Every file is already synced and the stable identity above survives the
    // close for the post-move replacement check.
    drop(dir);

    // Reject a pre-existing alias without touching it. A replacement racing the
    // check is moved as an inode into bound recovery and rejected there; it is
    // never followed for a write.
    let old_recovery = match root.symlink_metadata("publish") {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err((
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "static-publish output is not a real directory",
                    ),
                    None,
                ));
            }
            let recovery = reserve_publish_recovery(graph, &root).map_err(|error| (error, None))?;
            publish_recovery_race_hook(&recovery).map_err(|error| (error, None))?;
            root.rename("publish", &recovery.dir, "previous")
                .map_err(|error| (error, None))?;
            let previous = recovery.path.join("previous");
            let retired = recovery
                .dir
                .symlink_metadata("previous")
                .map_err(|error| (error, Some(previous.clone())))?;
            if !retired.is_dir() || retired.file_type().is_symlink() {
                return Err((
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "static-publish output changed during retirement",
                    ),
                    Some(previous),
                ));
            }
            Some(recovery)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err((error, None)),
    };

    let previous_kept = old_recovery
        .as_ref()
        .map(|recovery| recovery.path.join("previous"));

    if let Err(error) = crate::model::move_file_noreplace(&path, out) {
        // The previous site stays complete in conflict recovery. Avoid a
        // compare-then-replace restoration that could clobber a late winner.
        return Err((error, previous_kept));
    }
    let out_meta = fs::symlink_metadata(out).map_err(|error| (error, previous_kept.clone()))?;
    let same_stage = out_meta.is_dir()
        && !out_meta.file_type().is_symlink()
        && identity_from_path(out).is_ok_and(|live| live == identity);
    if same_stage {
        return Ok(());
    }

    // A replaced stage must never remain live. Move it through the bound graph
    // and recovery directory handles; the previous complete site is already
    // retained separately and is not overwritten during automatic recovery.
    let bad =
        reserve_publish_recovery(graph, &root).map_err(|error| (error, previous_kept.clone()))?;
    let _ = root.rename("publish", &bad.dir, "invalid-stage");
    Err((
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "static-publish staging directory changed during commit",
        ),
        previous_kept,
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn roots(label: &str) -> (PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("tine-publish-{label}-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!(
            "tine-publish-{label}-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&outside).unwrap();
        (base, outside)
    }

    #[test]
    fn publish_commit_never_writes_through_a_replaced_output_symlink() {
        let (base, outside) = roots("output-swap");
        fs::write(outside.join("index.html"), "outside sentinel").unwrap();
        let graph = Graph::open(&base);
        let stage = reserve_publish_stage(&graph).unwrap();
        write_publish_stage_file(&stage, "index.html", b"generated").unwrap();
        symlink(&outside, base.join("publish")).unwrap();
        assert!(commit_publish_stage_report(&graph, stage, &base.join("publish")).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("index.html")).unwrap(),
            "outside sentinel"
        );
        assert!(fs::symlink_metadata(base.join("publish"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn publish_stage_handle_survives_ambient_symlink_swap_without_outside_write() {
        let (base, outside) = roots("stage-swap");
        fs::write(outside.join("style.css"), "outside sentinel").unwrap();
        PUBLISH_STAGE_WRITE_SWAP.with(|slot| *slot.borrow_mut() = Some(outside.clone()));
        let store = Store::open(&base, Default::default()).unwrap().0;
        let result = store.publish_site(&mut |writer| {
            writer.write("style.css", b"generated")?;
            writer.write("public.html", b"generated page")
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(outside.join("style.css")).unwrap(),
            "outside sentinel"
        );
        assert!(!outside.join("public.html").exists());
    }

    #[test]
    fn publish_recovery_handle_survives_ambient_symlink_swap_without_outside_move() {
        let (base, outside) = roots("recovery-swap");
        fs::create_dir_all(base.join("publish")).unwrap();
        fs::write(base.join("publish/index.html"), "previous site").unwrap();
        fs::write(outside.join("previous"), "outside sentinel").unwrap();
        PUBLISH_RECOVERY_SWAP.with(|slot| *slot.borrow_mut() = Some(outside.clone()));
        let store = Store::open(&base, Default::default()).unwrap().0;
        let receipt = store
            .publish_site(&mut |writer| writer.write("index.html", b"generated"))
            .unwrap();
        assert!(receipt.site.join("index.html").exists());
        assert_eq!(
            fs::read_to_string(outside.join("previous")).unwrap(),
            "outside sentinel"
        );
        let conflicts = base.join("logseq/.tine-trash/conflicts");
        assert!(fs::read_dir(conflicts)
            .unwrap()
            .flatten()
            .any(|entry| entry.path().join("previous/index.html").exists()));
    }
}
