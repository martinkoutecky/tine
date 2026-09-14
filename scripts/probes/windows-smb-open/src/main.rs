use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _};
use cap_std::{ambient_authority, fs::{Dir, OpenOptions}};
use std::{fs, io::{self, Read}, path::Path};

fn step<T>(label: &str, result: io::Result<T>) -> io::Result<T> {
    match &result {
        Ok(_) => println!("PASS {label}"),
        Err(e) => println!("FAIL {label}: kind={:?} raw_os_error={:?} detail={e}", e.kind(), e.raw_os_error()),
    }
    result
}

// Exact options and post-open checks of model.rs::open_projection_dir_nofollow
// on Windows. This is a disposable diagnostic, never a production fallback.
fn checked_dir(dir: &Dir, name: &str) -> io::Result<Dir> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).maybe_dir(true);
    let file = step(&format!("cap open_with directory {name:?} [read,no-follow,maybe-dir]"), dir.open_with(name, &options))?.into_std();
    let metadata = step("std file.metadata after capability directory open", file.metadata())?;
    if !metadata.is_dir() {
        return Err(io::Error::other("not a directory"));
    }
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt as _;
        if metadata.file_attributes() & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other("directory is a reparse point"));
        }
    }
    #[cfg(windows)] {
        use std::os::windows::io::AsRawHandle as _;
        use windows_sys::Win32::Storage::FileSystem::{FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO};
        let mut info = FILE_ID_INFO::default();
        let result = unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo, (&mut info as *mut FILE_ID_INFO).cast(), std::mem::size_of::<FILE_ID_INFO>() as u32) };
        step("GetFileInformationByHandleEx(FileIdInfo) directory identity", if result == 0 { Err(io::Error::last_os_error()) } else { Ok(()) })?;
        println!("IDENTITY volume={} id={:?}", info.VolumeSerialNumber, info.FileId.Identifier);
    }
    Ok(Dir::from_std_file(file))
}

fn probe(root: &Path) -> io::Result<()> {
    println!("PROBE root={}", root.display());
    let expected = b"- generated NAS read control\n";
    // Only the unique generated fixture supplied by run.ps1 is accepted.
    if !root.file_name().is_some_and(|n| n.to_string_lossy().starts_with("tine-smb-533-")) {
        return Err(io::Error::other("expected generated fixture root"));
    }
    step("ambient create pages", fs::create_dir_all(root.join("pages")))?;
    step("ambient write pages/Probe.md", fs::write(root.join("pages/Probe.md"), expected))?;
    let bytes = step("ambient read same file", fs::read(root.join("pages/Probe.md")))?;
    assert_eq!(bytes, expected);
    let canonical = step("canonicalize graph root", fs::canonicalize(root))?;
    println!("CANONICAL {}", canonical.display());
    let parent = step("canonicalize graph parent", fs::canonicalize(root.parent().unwrap()))?;
    let parent = step("cap Dir::open_ambient_dir canonical parent", Dir::open_ambient_dir(parent, ambient_authority()))?;
    let retained = checked_dir(&parent, root.file_name().unwrap().to_str().unwrap())?;
    let entries = step("cap graph entries", retained.entries())?;
    for e in entries { let e=step("cap graph entry", e)?; println!("ENTRY {:?}", e.file_name()); }
    step("cap graph symlink_metadata pages", retained.symlink_metadata("pages"))?;
    let pages = checked_dir(&retained,"pages")?;
    step("cap pages symlink_metadata Probe.md", pages.symlink_metadata("Probe.md"))?;
    let mut opts=OpenOptions::new();opts.read(true).follow(FollowSymlinks::No);
    let mut file=step("cap pages open_with Probe.md [read,no-follow]", pages.open_with("Probe.md",&opts))?;
    let mut bytes=Vec::new();step("cap file read_to_end",file.read_to_end(&mut bytes))?;assert_eq!(bytes,expected);
    let bytes=step("cap graph read pages/Probe.md",retained.read("pages/Probe.md"))?;assert_eq!(bytes,expected);
    step("ambient create nested Unicode control", fs::create_dir_all(root.join("pages/nested folder")))?;
    step("ambient write nested Unicode control", fs::write(root.join("pages/nested folder/图谱.md"), expected))?;
    let nested = checked_dir(&pages, "nested folder")?;
    let bytes = step("cap nested Unicode file read", nested.read("图谱.md"))?;
    assert_eq!(bytes, expected);
    let link_meta = step("cap symlink_metadata of known outside junction", retained.symlink_metadata("outside-link"))?;
    assert!(link_meta.file_type().is_symlink(), "refusal control must really be a link");
    assert!(checked_dir(&retained, "outside-link").is_err(), "no-follow boundary accepted outside junction");
    println!("PASS no-follow refuses known outside junction");
    println!("COMPLETE root={}",root.display());
    Ok(())
}
fn main() {
    let mut failed=false;
    for root in std::env::args_os().skip(1) { if let Err(e)=probe(Path::new(&root)){println!("PROBE_FAILED {e}");failed=true;} }
    if failed { std::process::exit(1); }
}
