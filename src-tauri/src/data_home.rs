//! Keep the app-data home usable, so an unwritable one degrades instead of
//! killing the app at launch (port of master 8e1ea0bfd, final shape at master
//! HEAD).
//!
//! **Question answered.** [`ensure_usable`] — before anything resolves the
//! app-data dir, can Tine (and Tauri, and WebKitGTK) write under it? If not,
//! move the data home for this launch to the first writable fallback
//! (`~/.tine-data`, `$XDG_RUNTIME_DIR/tine-data`, a uid-namespaced temp dir) by
//! setting `XDG_DATA_HOME`, the one lever that moves Tauri's path resolver,
//! WebKitGTK's website-data manager and Tine's own settings together.
//! [`take_data_home_fallback_notice`] hands the fallback path to the frontend
//! once, so the relocation is never silent (I-9).
//!
//! **Why before Tauri.** Tauri creates the WebView data dir inside its own
//! `setup()` while building the configured windows and panics on the IO error
//! (`Failed to setup app: Permission denied`) before any Tine hook runs, so a
//! root-owned `~/.local/share` is a crash loop with nothing to act on.
//!
//! **Refusal.** If no candidate is writable, the launch prints one sentence
//! naming the failure family and exits 1 (scenario: disk error / a filesystem
//! the user cannot write — docs/storage-contract.md, `src-tauri::data_home`).
//! Relocation is desktop Linux only. All five targets classify the resolved
//! app-data store at setup and warn about detected volatile storage. Linux and
//! Android use statfs magic; Apple recognizes memory filesystem names, Windows
//! recognizes RAM drives, and all detect temporary/runtime paths. RAM disk
//! images presenting as ordinary filesystems and probe errors remain unverified.
//! Cost: one unnamed temporary file for the Linux writability probe plus one
//! metadata filesystem probe per launch; nothing persisted by classification.

#[cfg(all(desktop, target_os = "linux"))]
use crate::debug::{diag, diag_private};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The fallback data home iff this launch relocated; taken once by the command.
static RELOCATED_TO: Mutex<Option<String>> = Mutex::new(None);
static VOLATILE: Mutex<bool> = Mutex::new(false);

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct DataHomeNotice {
    relocated_to: Option<String>,
    volatile: bool,
}

/// Classify the actual Tauri app-data location, including ordinary launches
/// with XDG_DATA_HOME on tmpfs. This is advisory and never gates graph open.
/// A negative probe is unverified storage, not proof of crash durability.
pub(crate) fn classify_at_launch(dir: &Path) {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty());
    *VOLATILE.lock().unwrap_or_else(|p| p.into_inner()) = volatile_location(
        dir,
        &std::env::temp_dir(),
        runtime.as_deref().map(Path::new),
        volatile_filesystem,
    );
}

fn volatile_location(
    dir: &Path,
    temp: &Path,
    runtime: Option<&Path>,
    probe: impl FnOnce(&Path) -> std::io::Result<bool>,
) -> bool {
    // Setup may run before the app-specific directory is created. Resolve
    // existing ancestors so a new app folder inherits its real mount's policy.
    let resolve = |path: &Path| {
        path.ancestors()
            .find_map(|ancestor| {
                std::fs::canonicalize(ancestor)
                    .ok()
                    .map(|resolved| resolved.join(path.strip_prefix(ancestor).expect("ancestor")))
            })
            .unwrap_or_else(|| path.to_path_buf())
    };
    let dir = resolve(dir);
    dir.starts_with(resolve(temp))
        || runtime.is_some_and(|runtime| dir.starts_with(resolve(runtime)))
        || probe(dir.ancestors().find(|path| path.is_dir()).unwrap_or(&dir)).unwrap_or(false)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn volatile_filesystem(dir: &Path) -> std::io::Result<bool> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes())?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(volatile_magic(unsafe { info.assume_init() }.f_type as u64))
}

#[cfg(any(target_os = "linux", target_os = "android", test))]
fn volatile_magic(magic: u64) -> bool {
    // Linux/Android TMPFS_MAGIC and RAMFS_MAGIC, respectively.
    matches!(magic as u32, 0x0102_1994 | 0x8584_58f6)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn volatile_filesystem(dir: &Path) -> std::io::Result<bool> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes())?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    let name = unsafe { std::ffi::CStr::from_ptr(info.f_fstypename.as_ptr()) };
    Ok(matches!(name.to_bytes(), b"tmpfs" | b"ramfs" | b"mfs"))
}

#[cfg(target_os = "windows")]
fn volatile_filesystem(dir: &Path) -> std::io::Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut volume = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(path.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // DRIVE_RAMDISK. Other drive types (including APFS-backed RAM images on
    // Apple) cannot establish persistence; no positive guarantee is inferred.
    Ok(unsafe { GetDriveTypeW(volume.as_ptr()) } == 6)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
)))]
compile_error!("app-data durability classification needs an explicit platform policy");

/// Can files be created under `base` the way Tauri and WebKitGTK are about to?
/// `create_dir_all` alone succeeds on an existing directory nobody may write,
/// which is exactly the reported case, so create an unnamed temporary file in
/// the app dir when it exists (its own mode matters then) and in the base
/// otherwise. `tempfile_in` uses `O_TMPFILE` where the filesystem has it (no
/// name ever exists, so a crash mid-probe leaves nothing behind) and otherwise
/// creates and immediately unlinks a random name; nothing is persisted.
#[cfg(all(desktop, target_os = "linux"))]
fn probe_writable(base: &Path, identifier: &str) -> std::io::Result<()> {
    let app_dir = base.join(identifier);
    let target = if app_dir.is_dir() {
        app_dir
    } else {
        base.to_path_buf()
    };
    std::fs::create_dir_all(&target)?;
    tempfile::tempfile_in(&target).map(drop)
}

/// Fallback data homes, best first: the home directory (survives a reboot),
/// then the private runtime dir, then a temp dir namespaced by the home
/// directory's owner so another account cannot squat it.
#[cfg(all(desktop, target_os = "linux"))]
fn fallback_candidates() -> Vec<PathBuf> {
    use std::os::unix::fs::MetadataExt;

    let mut candidates = Vec::new();
    let home = dirs::home_dir();
    if let Some(home) = &home {
        candidates.push(home.join(".tine-data"));
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty()) {
        candidates.push(PathBuf::from(runtime).join("tine-data"));
    }
    if let Some(uid) = home
        .as_ref()
        .and_then(|home| std::fs::metadata(home).ok())
        .map(|meta| meta.uid())
    {
        candidates.push(std::env::temp_dir().join(format!("tine-data-{uid}")));
    }
    candidates
}

/// Probe the app-data home and relocate it for this launch if it cannot be
/// written. Call from `run()` before anything reads settings and before the
/// Tauri Builder, while no other thread reads the environment.
#[cfg(all(desktop, target_os = "linux"))]
pub(crate) fn ensure_usable(identifier: &str) {
    let Some(base) = dirs::data_dir() else {
        return;
    };
    let error = match probe_writable(&base, identifier) {
        Ok(()) => return,
        Err(error) => error,
    };
    diag_private(
        "app-data-home-unwritable",
        format!("app-data home {} is not writable ({error})", base.display()),
    );
    for candidate in fallback_candidates() {
        if probe_writable(&candidate, identifier).is_err() {
            continue;
        }
        std::env::set_var("XDG_DATA_HOME", &candidate);
        diag_private(
            "app-data-home-relocated",
            format!(
                "app-data home relocated to {} for this launch",
                candidate.display()
            ),
        );
        *RELOCATED_TO
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(candidate.display().to_string());
        return;
    }
    diag("app-data-home-none-writable");
    eprintln!("{}", fatal_no_data_home_message(error.kind()));
    std::process::exit(1);
}

/// The always-on fatal line: names the failure family (I-9: the user cannot
/// relaunch past `exit(1)` to read a debug log) by the bounded `ErrorKind`,
/// never OS prose or the directory (I-5).
#[cfg(all(desktop, target_os = "linux"))]
fn fatal_no_data_home_message(kind: std::io::ErrorKind) -> String {
    format!(
        "Tine cannot start: it has nowhere to keep its application data ({kind:?}).\n\
         Fix the permissions on the configured application-data directory, \n\
         or set XDG_DATA_HOME to a directory you can write."
    )
}

#[cfg(not(all(desktop, target_os = "linux")))]
pub(crate) fn ensure_usable(_identifier: &str) {}

/// Command: relocation and detected volatile storage ONCE per launch.
#[tauri::command]
pub(crate) fn take_data_home_fallback_notice() -> Option<DataHomeNotice> {
    let relocated_to = RELOCATED_TO
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    let volatile = std::mem::take(&mut *VOLATILE.lock().unwrap_or_else(|p| p.into_inner()));
    (relocated_to.is_some() || volatile).then_some(DataHomeNotice {
        relocated_to,
        volatile,
    })
}

#[cfg(all(test, desktop, target_os = "linux"))]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tine-data-home-{}-{tag}", std::process::id()))
    }

    fn set_mode(dir: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn a_writable_base_probes_clean_and_leaves_nothing_behind() {
        let base = tmp("clean");
        std::fs::create_dir_all(&base).unwrap();
        probe_writable(&base, "test.app.identifier").unwrap();
        let residue: Vec<_> = std::fs::read_dir(&base).unwrap().flatten().collect();
        assert!(residue.is_empty(), "probe left {residue:?} behind");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn an_unwritable_base_is_rejected_even_though_create_dir_all_succeeds() {
        let base = tmp("ro-base");
        std::fs::create_dir_all(&base).unwrap();
        set_mode(&base, 0o555);
        // The exact trap: this is what Tauri relies on, and it reports success.
        assert!(std::fs::create_dir_all(&base).is_ok());
        let error = probe_writable(&base, "test.app.identifier").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        set_mode(&base, 0o755);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn an_unwritable_existing_app_dir_is_rejected_under_a_writable_base() {
        let base = tmp("ro-app");
        let app = base.join("test.app.identifier");
        std::fs::create_dir_all(&app).unwrap();
        set_mode(&app, 0o555);
        assert!(probe_writable(&base, "test.app.identifier").is_err());
        set_mode(&app, 0o755);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn fatal_data_home_message_keeps_error_kind_without_prose_or_path() {
        let denied = std::io::Error::from_raw_os_error(13);
        let message = fatal_no_data_home_message(denied.kind());
        assert!(message.contains("PermissionDenied"), "I-9: {message}");
        assert!(
            !message.contains(&denied.to_string()) && !message.contains("os error"),
            "I-5: no OS prose: {message}"
        );
        assert!(!message.contains('/'), "I-5: no directory: {message}");
        assert!(fatal_no_data_home_message(std::io::ErrorKind::NotFound).contains("NotFound"));
    }

    #[test]
    fn the_notice_is_delivered_once() {
        *RELOCATED_TO.lock().unwrap() = Some("/tmp/example".to_string());
        *VOLATILE.lock().unwrap() = true;
        assert_eq!(
            take_data_home_fallback_notice(),
            Some(DataHomeNotice {
                relocated_to: Some("/tmp/example".to_string()),
                volatile: true
            })
        );
        assert_eq!(take_data_home_fallback_notice(), None);
    }
}

#[cfg(test)]
mod durability_tests {
    use super::*;

    #[test]
    fn sb1_data_home_classifies_memory_and_temporary_storage_without_refusing() {
        assert!(volatile_magic(0x0102_1994));
        assert!(volatile_magic(0x8584_58f6));
        assert!(!volatile_magic(0xef53));
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("app-data");
        let temp = dir.path().join("temp");
        let runtime = dir.path().join("runtime");
        assert!(volatile_location(&data, &temp, None, |_| Ok(true)));
        assert!(!volatile_location(&data, &temp, None, |_| Ok(false)));
        // A failed probe never blocks opening; it establishes no guarantee.
        assert!(!volatile_location(&data, &temp, None, |_| Err(
            std::io::ErrorKind::PermissionDenied.into()
        )));
        assert!(volatile_location(&temp.join("data"), &temp, None, |_| Ok(
            false
        )));
        assert!(volatile_location(
            &runtime.join("data"),
            &temp,
            Some(&runtime),
            |_| Ok(false)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sb1_data_home_real_tmpfs_probe() {
        let shm = Path::new("/dev/shm");
        if !shm.is_dir() || !volatile_filesystem(shm).unwrap() {
            eprintln!(
                "no /dev/shm tmpfs available; fake-statfs classification is covered separately"
            );
            return;
        }
        assert!(volatile_location(
            shm,
            Path::new("/not-the-temp-directory"),
            None,
            volatile_filesystem
        ));
        let unborn = shm.join(format!("tine-sb1-not-created-{}", uuid::Uuid::new_v4()));
        assert!(!unborn.exists());
        assert!(volatile_location(
            &unborn,
            Path::new("/not-the-temp-directory"),
            None,
            volatile_filesystem
        ));
    }

    #[test]
    fn sb1_data_home_platform_policy_names_all_shipped_targets() {
        let source = include_str!("data_home.rs");
        let production = &source[..source.find("#[cfg(test)]\nmod durability_tests").unwrap()];
        for target in ["linux", "android", "macos", "ios", "windows"] {
            assert!(production.contains(&format!("target_os = \"{target}\"")));
        }
        assert!(production.contains("libc::statfs("));
        assert!(production.contains("GetDriveTypeW("));
        assert!(production.contains("f_fstypename"));
        assert!(include_str!("lib.rs").contains("data_home::classify_at_launch(&dir)"));
    }
}
