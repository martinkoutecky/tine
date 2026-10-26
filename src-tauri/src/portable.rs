//! Portable Windows Tine (GH #719): app state beside the executable.
//!
//! **Question answered.** [`mode`]: is this process a portable instance, and
//! if so, where does it keep its app state? A Windows build is portable iff
//! all of these hold, each read without following links:
//!
//! - a real directory named [`PORTABLE_FOLDER`] (`tine-data`) sits beside the
//!   running executable: not a file, not a symbolic link, not a junction;
//! - no NSIS uninstaller (`uninstall.exe`, written by Tauri's NSIS template
//!   beside the main binary in `$INSTDIR`) sits beside it;
//! - the process has no MSIX package identity.
//!
//! Anything missing, or any probe that cannot answer, means an installed
//! build. The portable ZIP ships the folder; the NSIS installer and the MSIX
//! package never do, so installed builds are unchanged, and a stray
//! `tine-data` folder in an install directory cannot redirect it.
//!
//! **Layout.** Mirrors the two per-user folders an installed Windows Tine uses,
//! one level per identity so a stable and a Beta exe in the same folder never
//! share state (docs/app-identity.md):
//!
//! ```text
//! <exe dir>\tine-data\<identifier>\roaming      app_config_dir = app_data_dir
//!                                              (installed: %APPDATA%\<identifier>)
//! <exe dir>\tine-data\<identifier>\local        app_local_data_dir = app_cache_dir,
//!                                              WebView2 user data (EBWebView)
//!                                              (installed: %LOCALAPPDATA%\<identifier>)
//! <exe dir>\tine-data\<identifier>\local\logs   app_log_dir, opt-in debug log
//! ```
//!
//! **One owner.** [`apply`] hands the layout to Tauri's own
//! `app > appDirectoriesOverride`, so every `app.path().app_*_dir()` call and
//! the WebView2 user-data folder Tauri derives from `app_local_data_dir` follow
//! it; [`crate::app_identity::current_app_data_dir`] (settings read before the
//! Builder exists) asks [`dirs`] first. The production lookups that resolve a
//! host folder another way are listed call site by call site in
//! `host_folder_lookups_go_through_the_portable_owner`.
//!
//! **Refusal, never a second home.** A portable folder Tine cannot write (a
//! write-protected stick, a read-only share; in-scope threat: a disk error or
//! a filesystem the user cannot write) stops the launch in [`preflight`],
//! before anything is written: a native error names the folder and says to
//! move Tine somewhere writable. Falling back to the installed profile would
//! split state across two homes, and the next launch from a writable folder
//! would silently leave the newer half behind (GH #719 review r1). Portable
//! identity (no installer) holds whether or not its storage is usable.
//! Nothing under `%APPDATA%` / `%LOCALAPPDATA%` is read, moved or deleted by
//! portable mode.
//!
//! Cost: three metadata reads and, when portable, one unnamed temporary file
//! per folder per launch (released at once). Nothing new is persisted.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The folder beside the executable that makes a Windows build portable.
pub(crate) const PORTABLE_FOLDER: &str = "tine-data";

/// The file Tauri's NSIS installer writes beside the main binary
/// (`WriteUninstaller "$INSTDIR\uninstall.exe"`, tauri-bundler 2.x
/// `bundle/windows/nsis/installer.nsi`): an installed copy.
pub(crate) const NSIS_UNINSTALLER: &str = "uninstall.exe";

/// Where a portable instance keeps its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PortableDirs {
    /// Settings, sessions, backups, plugins, diagnostics, window state.
    pub(crate) roaming: PathBuf,
    /// WebView2 user data (localStorage, IndexedDB, caches) and Tine's caches.
    pub(crate) local: PathBuf,
}

impl PortableDirs {
    fn under(folder: &Path, identifier: &str) -> Self {
        let base = folder.join(identifier);
        Self {
            roaming: base.join("roaming"),
            local: base.join("local"),
        }
    }

    pub(crate) fn logs(&self) -> PathBuf {
        self.local.join("logs")
    }

    /// Tauri's override for every app directory: the same split as an
    /// installed Windows build (config = data, local data = cache).
    pub(crate) fn app_directories(&self) -> tauri::utils::config::AppDirectoriesOverride {
        tauri::utils::config::AppDirectoriesOverride::Directories(
            tauri::utils::config::AppDirectoryOverrides {
                config: Some(self.roaming.clone()),
                data: Some(self.roaming.clone()),
                local_data: Some(self.local.clone()),
                cache: Some(self.local.clone()),
                log: Some(self.logs()),
            },
        )
    }
}

/// What the file system says beside the executable, read without following
/// links. Separated from the decision so tests drive every answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Facts {
    /// `tine-data` is a real directory (not a file, link or junction).
    pub(crate) real_folder: bool,
    /// `Some(false)` only when the uninstaller is affirmatively absent.
    pub(crate) uninstaller: Option<bool>,
    /// MSIX package identity: `Some(false)` only when affirmatively none.
    pub(crate) packaged: Option<bool>,
}

/// How this process runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The platform's per-user folders.
    Installed,
    /// Every app directory in `tine-data` beside the exe.
    Portable(PortableDirs),
    /// Portable, but the folder cannot take a file: [`preflight`] refuses to
    /// start, so nothing lands anywhere else.
    Unwritable(PortableDirs, String),
}

/// Is a real directory (no link, no junction) at `path`? Any error: no.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn real_directory(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(target_os = "windows")]
    {
        // A junction or any other reparse point is not the portable folder.
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    metadata.file_type().is_dir() && !metadata.file_type().is_symlink()
}

/// Is anything at `path` (without following links)? An unanswerable probe is
/// `None`, which the decision reads as installed.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn present(path: &Path) -> Option<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Some(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// Read the facts beside `exe` from the file system.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn observe(exe: &Path, packaged: Option<bool>) -> Option<Facts> {
    let beside = exe.parent()?;
    Some(Facts {
        real_folder: real_directory(&beside.join(PORTABLE_FOLDER)),
        uninstaller: present(&beside.join(NSIS_UNINSTALLER)),
        packaged,
    })
}

/// The portable layout for `exe`, if the facts make it portable.
fn resolve(exe: &Path, identifier: &str, facts: Facts) -> Option<PortableDirs> {
    let portable =
        facts.real_folder && facts.uninstaller == Some(false) && facts.packaged == Some(false);
    let folder = exe.parent()?.join(PORTABLE_FOLDER);
    portable.then(|| PortableDirs::under(&folder, identifier))
}

/// Can Tine, Tauri and WebView2 create files in both portable folders?
/// Creating an existing read-only folder succeeds, so create an unnamed
/// temporary file in each as well (nothing persists). A refused
/// probe removes exactly the folders it created (newest first), keeping
/// anything that was already there (GH #719 review r2).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn usable(dirs: &PortableDirs) -> std::io::Result<()> {
    let mut created = Vec::new();
    let probed = probe_folders(dirs, &mut created);
    if probed.is_err() {
        for dir in created.iter().rev() {
            // `remove_dir` only removes an empty folder: never anything else.
            let _ = std::fs::remove_dir(dir);
        }
    }
    probed
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn probe_folders(dirs: &PortableDirs, created: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for dir in [&dirs.roaming, &dirs.local] {
        create_recording(dir, created)?;
        tempfile::tempfile_in(dir).map(drop)?;
    }
    Ok(())
}

/// `create_dir_all`, recording each folder this call created.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn create_recording(dir: &Path, created: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let missing: Vec<&Path> = dir
        .ancestors()
        .take_while(|ancestor| std::fs::symlink_metadata(ancestor).is_err())
        .collect();
    for ancestor in missing.into_iter().rev() {
        match std::fs::create_dir(ancestor) {
            Ok(()) => created.push(ancestor.to_path_buf()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    if std::fs::metadata(dir)?.is_dir() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} is not a folder", dir.display()),
        ))
    }
}

/// Decide the mode for `exe`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn decide(
    exe: &Path,
    identifier: &str,
    facts: Facts,
    usable: impl Fn(&PortableDirs) -> std::io::Result<()>,
) -> Mode {
    match resolve(exe, identifier, facts) {
        None => Mode::Installed,
        Some(dirs) => match usable(&dirs) {
            Ok(()) => Mode::Portable(dirs),
            Err(error) => Mode::Unwritable(dirs, error.to_string()),
        },
    }
}

static MODE: OnceLock<Mode> = OnceLock::new();

#[cfg(target_os = "windows")]
fn detect() -> Mode {
    let Ok(exe) = std::env::current_exe() else {
        return Mode::Installed;
    };
    match observe(&exe, crate::platform::packaged()) {
        Some(facts) => decide(&exe, crate::app_identity::APP_IDENTIFIER, facts, usable),
        None => Mode::Installed,
    }
}

/// Only the Windows ZIP is a portable artifact. Linux AppImages, macOS bundles
/// and the mobile apps keep the platform's per-user folders.
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    target_os = "ios"
))]
fn detect() -> Mode {
    Mode::Installed
}

/// This process's mode, decided once.
pub(crate) fn mode() -> &'static Mode {
    MODE.get_or_init(detect)
}

/// The portable layout, also when it is unwritable: a portable instance never
/// resolves the installed profile (preflight stops an unwritable one first).
/// `None` on an installed build and on every platform but Windows.
pub(crate) fn dirs() -> Option<&'static PortableDirs> {
    dirs_of(mode())
}

fn dirs_of(mode: &Mode) -> Option<&PortableDirs> {
    match mode {
        Mode::Installed => None,
        Mode::Portable(dirs) | Mode::Unwritable(dirs, _) => Some(dirs),
    }
}

/// Portable identity (no installer), whether or not its storage is usable.
pub(crate) fn active() -> bool {
    dirs().is_some()
}

/// The refusal text for an unwritable portable folder. English only: it is
/// shown before the app (and its translations) exist.
fn unwritable_message(dirs: &PortableDirs, error: &str) -> String {
    let folder = dirs
        .roaming
        .parent()
        .and_then(Path::parent)
        .unwrap_or(&dirs.roaming);
    format!(
        "{product} cannot write to its portable data folder:\n\n{folder}\n\n({error})\n\n\
         Move the whole {product} folder to a place you can write to (for example your \
         Documents folder or a writable USB drive) and start it again.\n\n\
         {product} did not start and wrote nothing anywhere else.",
        product = crate::app_identity::PRODUCT_NAME,
        folder = folder.display(),
    )
}

#[cfg(target_os = "windows")]
fn show_refusal(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |value: &str| value.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide(crate::app_identity::PRODUCT_NAME));
    // SAFETY: both buffers are NUL-terminated UTF-16 and outlive the call; a
    // null owner window is allowed.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn show_refusal(_text: &str) {}

/// GH #719: the first thing `main` (and `run`) does. An unwritable portable
/// folder ends the process here with a native error, before the debug log,
/// the CLI, settings or Tauri can write anywhere. Idempotent.
pub fn preflight() {
    if let Mode::Unwritable(dirs, error) = mode() {
        let text = unwritable_message(dirs, error);
        eprintln!("{text}");
        show_refusal(&text);
        std::process::exit(1);
    }
}

/// Point Tauri's app directories (and with them the WebView2 user data) at the
/// portable layout. No-op on an installed build.
pub(crate) fn apply(config: &mut tauri::Config) {
    if let Some(dirs) = dirs() {
        config.app.app_directories_override = Some(dirs.app_directories());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::utils::config::{AppDirectoriesOverride, AppDirectoryOverrides};

    fn exe() -> PathBuf {
        Path::new("E:")
            .join("Apps")
            .join("Tine")
            .join("Tine Beta.exe")
    }

    const PORTABLE: Facts = Facts {
        real_folder: true,
        uninstaller: Some(false),
        packaged: Some(false),
    };

    fn folder() -> PathBuf {
        exe().parent().unwrap().join("tine-data")
    }

    #[test]
    fn an_exe_without_the_folder_is_an_installed_build() {
        let facts = Facts {
            real_folder: false,
            ..PORTABLE
        };
        assert_eq!(resolve(&exe(), "example.AppBeta", facts), None);
        assert_eq!(
            decide(&exe(), "example.AppBeta", facts, |_| Ok(())),
            Mode::Installed
        );
    }

    #[test]
    fn the_folder_beside_the_exe_makes_it_portable_per_identity() {
        let dirs = resolve(&exe(), "example.AppBeta", PORTABLE).unwrap();
        assert_eq!(
            dirs.roaming,
            folder().join("example.AppBeta").join("roaming")
        );
        assert_eq!(dirs.local, folder().join("example.AppBeta").join("local"));
        // A stable exe beside it keeps its own state.
        let stable = resolve(&exe(), "example.App", PORTABLE).unwrap();
        assert_ne!(stable.roaming, dirs.roaming);
        assert_ne!(stable.local, dirs.local);
    }

    /// GH #719 r1: only an affirmative answer from every probe is portable.
    /// An installer's uninstaller, MSIX identity, or a probe that cannot
    /// answer keeps the installed build where it is.
    #[test]
    fn only_affirmative_facts_make_a_portable_build() {
        let installed = [
            (
                "an NSIS install",
                Facts {
                    uninstaller: Some(true),
                    ..PORTABLE
                },
            ),
            (
                "an unreadable uninstaller probe",
                Facts {
                    uninstaller: None,
                    ..PORTABLE
                },
            ),
            (
                "an MSIX package",
                Facts {
                    packaged: Some(true),
                    ..PORTABLE
                },
            ),
            (
                "an unanswered package probe",
                Facts {
                    packaged: None,
                    ..PORTABLE
                },
            ),
            (
                "no real folder",
                Facts {
                    real_folder: false,
                    ..PORTABLE
                },
            ),
        ];
        for (case, facts) in installed {
            assert_eq!(resolve(&exe(), "example.App", facts), None, "{case}");
        }
        assert!(resolve(&exe(), "example.App", PORTABLE).is_some());
    }

    /// The facts as the file system gives them, on a real disk.
    #[test]
    fn the_disk_probe_reads_the_folder_and_the_uninstaller_without_following_links() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("Tine.exe");
        let facts = |packaged| observe(&exe, packaged).unwrap();
        // Nothing beside the exe.
        assert_eq!(
            facts(Some(false)),
            Facts {
                real_folder: false,
                uninstaller: Some(false),
                packaged: Some(false)
            }
        );
        // A file named tine-data is not the folder.
        std::fs::write(temp.path().join(PORTABLE_FOLDER), b"").unwrap();
        assert!(!facts(Some(false)).real_folder);
        std::fs::remove_file(temp.path().join(PORTABLE_FOLDER)).unwrap();
        // The portable ZIP's layout.
        std::fs::create_dir(temp.path().join(PORTABLE_FOLDER)).unwrap();
        assert_eq!(facts(Some(false)), PORTABLE);
        // The same folder in an NSIS install directory.
        std::fs::write(temp.path().join(NSIS_UNINSTALLER), b"").unwrap();
        assert_eq!(facts(Some(false)).uninstaller, Some(true));
        assert_eq!(
            decide(&exe, "example.App", facts(Some(false)), |_| Ok(())),
            Mode::Installed,
            "a stray tine-data folder must not move an installed build"
        );
    }

    /// A linked `tine-data` would put state wherever the link points.
    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_not_the_portable_folder() {
        let temp = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let exe = temp.path().join("Tine.exe");
        std::os::unix::fs::symlink(elsewhere.path(), temp.path().join(PORTABLE_FOLDER)).unwrap();
        let facts = observe(&exe, Some(false)).unwrap();
        assert!(!facts.real_folder);
        assert_eq!(decide(&exe, "example.App", facts, usable), Mode::Installed);
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    }

    /// Every Tauri app directory, and so the WebView2 user-data folder Tauri
    /// derives from `app_local_data_dir`, is inside the portable folder.
    #[test]
    fn every_app_directory_resolves_inside_the_portable_folder() {
        let dirs = resolve(&exe(), "example.App", PORTABLE).unwrap();
        let AppDirectoriesOverride::Directories(AppDirectoryOverrides {
            config,
            data,
            local_data,
            cache,
            log,
        }) = dirs.app_directories()
        else {
            panic!("portable mode overrides each directory");
        };
        for path in [&config, &data, &local_data, &cache, &log] {
            let path = path.as_ref().expect("every app directory is overridden");
            assert!(path.starts_with(folder()), "{} escapes", path.display());
        }
        assert_eq!(config, data, "installed Windows: both are %APPDATA%\\<id>");
        assert_eq!(
            local_data, cache,
            "installed Windows: both are %LOCALAPPDATA%\\<id>"
        );
        assert_ne!(data, local_data);
        assert_eq!(log, Some(dirs.local.join("logs")));
    }

    /// GH #719 r1: an unwritable portable folder is refused, never swapped for
    /// the installed profile, and it keeps portable identity (no installer).
    #[test]
    fn an_unwritable_portable_folder_is_refused_and_stays_portable() {
        let decided = decide(&exe(), "example.App", PORTABLE, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        });
        let Mode::Unwritable(dirs, _) = &decided else {
            panic!("expected a refusal, got {decided:?}");
        };
        assert_eq!(dirs_of(&decided), Some(dirs), "never the installed profile");
        assert!(dirs.roaming.starts_with(folder()));
        let text = unwritable_message(dirs, "access denied");
        assert!(text.contains(&folder().display().to_string()), "{text}");
        assert!(text.contains("Move the whole"), "{text}");
        assert_eq!(dirs_of(&Mode::Installed), None);
    }

    /// The same refusal from a real read-only folder.
    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_on_disk_is_unwritable() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("Tine.exe");
        let folder = temp.path().join(PORTABLE_FOLDER);
        std::fs::create_dir(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let probe = tempfile::tempfile_in(&folder);
        let facts = observe(&exe, Some(false)).unwrap();
        let decided = decide(&exe, "example.App", facts, usable);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if probe.is_ok() {
            return; // running as root: a read-only mode does not refuse writes
        }
        assert!(matches!(decided, Mode::Unwritable(..)), "{decided:?}");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    }

    /// GH #719 r2: a refused probe leaves the folder as it found it. With a
    /// file where `local` belongs and no `roaming`, the probe creates
    /// `<id>` and `roaming`, then fails on `local`: it removes exactly those.
    #[test]
    fn a_refused_probe_removes_only_the_folders_it_created() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("Tine.exe");
        let folder = temp.path().join(PORTABLE_FOLDER);
        let base = folder.join("example.App");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("local"), b"in the way").unwrap();
        std::fs::write(folder.join("keep.txt"), b"mine").unwrap();
        let facts = observe(&exe, Some(false)).unwrap();
        let decided = decide(&exe, "example.App", facts, usable);
        assert!(matches!(decided, Mode::Unwritable(..)), "{decided:?}");
        assert!(
            !base.join("roaming").exists(),
            "the probe left roaming behind"
        );
        // What was already there stays: <id> existed, the file and keep.txt too.
        assert!(base.is_dir());
        assert_eq!(std::fs::read(base.join("local")).unwrap(), b"in the way");
        assert_eq!(std::fs::read(folder.join("keep.txt")).unwrap(), b"mine");

        // A roaming folder that was already there is kept.
        std::fs::create_dir(base.join("roaming")).unwrap();
        let decided = decide(&exe, "example.App", facts, usable);
        assert!(matches!(decided, Mode::Unwritable(..)), "{decided:?}");
        assert!(
            base.join("roaming").is_dir(),
            "an existing folder was removed"
        );
    }

    #[test]
    fn apply_sets_the_override_only_when_portable() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        assert_eq!(
            context.config().app.app_directories_override,
            None,
            "the shipped config must not move installed builds"
        );
        apply(context.config_mut());
        // The test binary has no tine-data folder beside it: installed.
        assert_eq!(dirs(), None);
        assert_eq!(context.config().app.app_directories_override, None);
    }

    #[test]
    fn the_probe_creates_both_folders_and_leaves_no_file() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("Tine.exe");
        std::fs::create_dir(temp.path().join(PORTABLE_FOLDER)).unwrap();
        let facts = observe(&exe, Some(false)).unwrap();
        let Mode::Portable(decided) = decide(&exe, "example.App", facts, usable) else {
            panic!("portable");
        };
        for dir in [&decided.roaming, &decided.local] {
            assert!(dir.is_dir());
            assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
        }
    }

    #[test]
    fn run_applies_the_portable_override_to_the_one_app_context() {
        let source = include_str!("lib.rs");
        let run = &source[source.find("pub fn run()").expect("run")..];
        let preflight = run
            .find("portable::preflight();")
            .expect("GH #719: run() refuses an unwritable portable folder first");
        let debug = run.find("debug_init();").expect("run starts the debug log");
        let context = run
            .find("let mut context = tauri::generate_context!();")
            .expect("run builds its context");
        let apply = run
            .find("portable::apply(context.config_mut());")
            .expect("GH #719: run() must apply the portable layout");
        let build = run.find(".build(context)").expect("run builds the app");
        assert!(preflight < debug && context < apply && apply < build);
        // main() refuses before the CLI can read or write settings.
        let main = include_str!("main.rs");
        let preflight = main
            .find("tine_lib::portable_preflight();")
            .expect("GH #719: main() runs the portable preflight");
        assert!(preflight < main.find("cli_dispatch()").expect("cli"));
    }

    /// GH #719 guard: a host folder resolved outside Tauri's app-directory API
    /// would bypass portable mode. Each production lookup is listed by call
    /// site (file + the code on its line) with why it cannot write the host
    /// from a portable instance; a stale entry fails too.
    #[test]
    fn host_folder_lookups_go_through_the_portable_owner() {
        const LOOKUPS: &[&str] = &[
            "dirs::",
            "env::temp_dir(",
            ".temp_dir()",
            ".home_dir()",
            ".data_dir()",
            ".local_data_dir()",
            ".config_dir()",
            ".cache_dir()",
            "tempfile::tempdir(",
            "tempfile::tempfile(",
            "tempfile::Builder",
            "NamedTempFile::new(",
            "\"APPDATA\"",
            "\"LOCALAPPDATA\"",
            "\"USERPROFILE\"",
            "\"HOME\"",
        ];
        // (file, the code on its line, why it cannot write the host when portable)
        const ALLOWED: &[(&str, &str, &str)] = &[
            ("app_identity.rs", "dirs::data_dir().map(|base| base.join(identifier))", "app_data_dir_of: current_app_data_dir asks portable::dirs() first; its other caller was the Beta config seed, deleted for stable"),
            ("migrate_identifier.rs", "dirs::data_dir().map(|base| base.join(crate::app_identity::RELEASE_IDENTIFIER))", "only run_early calls it, after returning when portable"),
            ("debug.rs", "std::env::temp_dir().join(\"tine-debug.log\")", "installed builds only: a portable instance uses local\\logs"),
            ("commands.rs", "(\"LOCALAPPDATA\", Some(\"Programs\")),", "read only: draw.io install discovery"),
            ("commands.rs", "let home = std::env::var_os(\"HOME\")", "Linux only, read only: draw.io Flatpak discovery"),
            ("spellcheck.rs", "std::env::var_os(\"HOME\")", "Linux only, read only: hunspell dictionaries"),
            ("platform.rs", "\"HOME\",", "a child-process environment keep-list, not a lookup"),
            ("data_home.rs", "let home = dirs::home_dir();", "Linux only"),
            ("data_home.rs", "std::env::temp_dir().join(format!(\"tine-data-{uid}\"))", "Linux only"),
            ("data_home.rs", "let Some(base) = dirs::data_dir() else {", "Linux only"),
            ("linux_window_identity.rs", ".or_else(dirs::data_dir)", "Linux only"),
            ("youtube_identity.rs", "tempfile::Builder::new().prefix(\"tine-youtube-\")", "Linux only"),
        ];
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut used = vec![false; ALLOWED.len()];
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let relative = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                // Test-only sources: `*_tests.rs` files and `*_tests/` modules.
                if !name.ends_with(".rs")
                    || relative
                        .split('/')
                        .any(|part| part.trim_end_matches(".rs").ends_with("_tests"))
                {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                for (line_number, line) in production_lines(&source) {
                    let code = line.split("//").next().unwrap_or("");
                    let lookup = LOOKUPS
                        .iter()
                        .copied()
                        .find(|lookup| code.contains(*lookup))
                        .or_else(|| host_base_directory(code).then_some("BaseDirectory::<host>"));
                    let Some(lookup) = lookup else { continue };
                    let allowed = ALLOWED
                        .iter()
                        .position(|(file, site, _)| relative == *file && code.contains(site));
                    match allowed {
                        Some(index) => used[index] = true,
                        None => offenders.push(format!(
                            "{relative}:{line_number}: {lookup}: {}",
                            code.trim()
                        )),
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "GH #719: resolve app state through app.path().app_*_dir() or \
             app_identity::current_app_data_dir(), so a portable instance keeps it \
             beside the exe; or list the call site here with the reason it cannot \
             write the host:\n{}",
            offenders.join("\n")
        );
        let stale: Vec<_> = ALLOWED
            .iter()
            .zip(&used)
            .filter(|(_, used)| !**used)
            .map(|((file, site, _), _)| format!("{file}: {site}"))
            .collect();
        assert!(
            stale.is_empty(),
            "stale allowlist entries:\n{}",
            stale.join("\n")
        );
    }

    /// A Tauri `BaseDirectory` that is not one of the app directories the
    /// portable override moves (`AppConfig`, `AppData`, `AppLocalData`,
    /// `AppCache`, `AppLog`), e.g. `BaseDirectory::Home`.
    fn host_base_directory(code: &str) -> bool {
        code.match_indices("BaseDirectory::").any(|(at, prefix)| {
            let rest = &code[at + prefix.len()..];
            rest.starts_with(char::is_uppercase) && !rest.starts_with("App")
        })
    }

    #[test]
    fn the_guard_sees_host_base_directories() {
        assert!(host_base_directory(
            "app.path().resolve(\"state\", BaseDirectory::Home)"
        ));
        assert!(host_base_directory("BaseDirectory::Temp"));
        assert!(!host_base_directory("BaseDirectory::AppData"));
        assert!(!host_base_directory("use tauri::path::BaseDirectory;"));
    }

    /// The lines of `source` outside `#[cfg(test)]` items. rustfmt closes a
    /// top-level item with `}` in column 0, which ends a skipped region.
    fn production_lines(source: &str) -> Vec<(usize, &str)> {
        let mut lines = Vec::new();
        let mut skipping = false;
        let mut pending_attribute = false;
        for (index, line) in source.lines().enumerate() {
            if skipping {
                if line == "}" {
                    skipping = false;
                }
                continue;
            }
            let trimmed = line.trim_start();
            if line.starts_with("#[cfg(test)]") || line.starts_with("#[cfg(all(test") {
                pending_attribute = true;
                continue;
            }
            if pending_attribute {
                if trimmed.starts_with("#[") {
                    continue;
                }
                pending_attribute = false;
                if !line.trim_end().ends_with(';') {
                    skipping = !line.trim_end().ends_with('}');
                }
                continue;
            }
            lines.push((index + 1, line));
        }
        lines
    }
}
