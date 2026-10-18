//! Portable Windows Tine (GH #719): app state beside the executable.
//!
//! **Question answered.** [`dirs`]: is this process a portable instance, and
//! if so, where does it keep its app state? A Windows build is portable iff a
//! folder named [`PORTABLE_FOLDER`] (`tine-data`) sits beside the running
//! executable. The portable ZIP ships that folder; the NSIS installer and the
//! MSIX package never do, so installed builds are unchanged.
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
//! Builder exists) asks [`dirs`] first. Code that resolves a host folder any
//! other way is pinned by `host_folder_lookups_go_through_the_portable_owner`.
//!
//! **Recovery, not refusal.** A `tine-data` folder Tine cannot write (a
//! write-protected stick, a copy under `Program Files`; in-scope threat: disk
//! error or a filesystem the user cannot write) would make Tauri panic while
//! creating the WebView2 folder, before any window. That launch instead uses
//! the installed per-user location and the sticky data-home notice names it
//! (I-9), exactly like `data_home`'s fallback on Linux. Nothing already under
//! `%APPDATA%` / `%LOCALAPPDATA%` is read, moved or deleted by portable mode.
//!
//! Cost: one directory probe and, when portable, one unnamed temporary file per
//! folder per launch (released at once). Nothing new is persisted.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The folder beside the executable that makes a Windows build portable.
pub(crate) const PORTABLE_FOLDER: &str = "tine-data";

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

/// The portable layout for `exe`, if a `tine-data` folder sits beside it.
fn resolve(exe: &Path, identifier: &str, is_dir: impl Fn(&Path) -> bool) -> Option<PortableDirs> {
    let folder = exe.parent()?.join(PORTABLE_FOLDER);
    is_dir(&folder).then(|| PortableDirs::under(&folder, identifier))
}

// `usable` and `decide` run only on Windows; their tests run everywhere.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
/// Can Tine, Tauri and WebView2 create files in both portable folders?
/// `create_dir_all` succeeds on an existing read-only folder, so create an
/// unnamed temporary file in each as well (nothing persists).
fn usable(dirs: &PortableDirs) -> std::io::Result<()> {
    for dir in [&dirs.roaming, &dirs.local] {
        std::fs::create_dir_all(dir)?;
        tempfile::tempfile_in(dir).map(drop)?;
    }
    Ok(())
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
/// Decide portable mode for `exe`: `Ok(Some)` portable, `Ok(None)` installed,
/// `Err(folder)` portable requested but unusable (recover to installed).
fn decide(
    exe: &Path,
    identifier: &str,
    is_dir: impl Fn(&Path) -> bool,
    usable: impl Fn(&PortableDirs) -> std::io::Result<()>,
) -> Result<Option<PortableDirs>, PathBuf> {
    match resolve(exe, identifier, is_dir) {
        None => Ok(None),
        Some(dirs) => match usable(&dirs) {
            Ok(()) => Ok(Some(dirs)),
            Err(_) => Err(dirs.roaming),
        },
    }
}

static DIRS: OnceLock<Option<PortableDirs>> = OnceLock::new();

#[cfg(target_os = "windows")]
fn detect() -> Option<PortableDirs> {
    let exe = std::env::current_exe().ok()?;
    match decide(
        &exe,
        crate::app_identity::APP_IDENTIFIER,
        Path::is_dir,
        usable,
    ) {
        Ok(dirs) => dirs,
        Err(_unusable) => {
            // Tell the user where this launch keeps its state instead (I-9).
            // No diag here: the debug log asks this module for its own path.
            if let Some(installed) =
                crate::app_identity::app_data_dir_of(crate::app_identity::APP_IDENTIFIER)
            {
                crate::data_home::record_relocation(installed.display().to_string());
            }
            None
        }
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
fn detect() -> Option<PortableDirs> {
    None
}

/// The portable layout of this process, decided once. `None` on an installed
/// build and on every platform but Windows.
pub(crate) fn dirs() -> Option<&'static PortableDirs> {
    DIRS.get_or_init(detect).as_ref()
}

pub(crate) fn active() -> bool {
    dirs().is_some()
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

    #[test]
    fn an_exe_without_the_folder_is_an_installed_build() {
        assert_eq!(resolve(&exe(), "example.AppBeta", |_| false), None);
        assert_eq!(
            decide(&exe(), "example.AppBeta", |_| false, |_| Ok(())),
            Ok(None)
        );
    }

    #[test]
    fn the_folder_beside_the_exe_makes_it_portable_per_identity() {
        let folder = exe().parent().unwrap().join("tine-data");
        let probed = std::cell::RefCell::new(Vec::new());
        let dirs = resolve(&exe(), "example.AppBeta", |path| {
            probed.borrow_mut().push(path.to_path_buf());
            path == folder
        })
        .unwrap();
        assert_eq!(*probed.borrow(), vec![folder.clone()]);
        assert_eq!(dirs.roaming, folder.join("example.AppBeta").join("roaming"));
        assert_eq!(dirs.local, folder.join("example.AppBeta").join("local"));
        // A stable exe beside it keeps its own state.
        let stable = resolve(&exe(), "example.App", |path| path == folder).unwrap();
        assert_ne!(stable.roaming, dirs.roaming);
        assert_ne!(stable.local, dirs.local);
    }

    /// Every Tauri app directory, and so the WebView2 user-data folder Tauri
    /// derives from `app_local_data_dir`, is inside the portable folder.
    #[test]
    fn every_app_directory_resolves_inside_the_portable_folder() {
        let folder = exe().parent().unwrap().join("tine-data");
        let dirs = resolve(&exe(), "example.App", |path| path == folder).unwrap();
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
            assert!(path.starts_with(&folder), "{} escapes", path.display());
        }
        assert_eq!(config, data, "installed Windows: both are %APPDATA%\\<id>");
        assert_eq!(
            local_data, cache,
            "installed Windows: both are %LOCALAPPDATA%\\<id>"
        );
        assert_ne!(data, local_data);
        assert_eq!(log, Some(dirs.local.join("logs")));
    }

    #[test]
    fn an_unwritable_portable_folder_recovers_to_the_installed_location() {
        let folder = exe().parent().unwrap().join("tine-data");
        let decided = decide(
            &exe(),
            "example.App",
            |path| path == folder,
            |_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
        );
        assert_eq!(decided, Err(folder.join("example.App").join("roaming")));
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
        let decided = decide(&exe, "example.App", Path::is_dir, usable)
            .unwrap()
            .unwrap();
        for dir in [&decided.roaming, &decided.local] {
            assert!(dir.is_dir());
            assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
        }
    }

    #[test]
    fn run_applies_the_portable_override_to_the_one_app_context() {
        let source = include_str!("lib.rs");
        let run = &source[source.find("pub fn run()").expect("run")..];
        let context = run
            .find("let mut context = tauri::generate_context!();")
            .expect("run builds its context");
        let apply = run
            .find("portable::apply(context.config_mut());")
            .expect("GH #719: run() must apply the portable layout");
        let build = run.find(".build(context)").expect("run builds the app");
        assert!(context < apply && apply < build);
    }

    /// GH #719 guard: a host folder resolved outside Tauri's app-directory API
    /// would bypass portable mode. Each production lookup is listed with why it
    /// cannot write the host from a portable instance.
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
        ];
        const ALLOWED: &[(&str, &str)] = &[
            ("portable.rs", "the owner"),
            ("app_identity.rs", "asks portable::dirs() first"),
            (
                "migrate_identifier.rs",
                "run_early is a no-op when portable",
            ),
            (
                "experiment_config_seed.rs",
                "seed_from_release_once is a no-op when portable",
            ),
            ("data_home.rs", "Linux only"),
            ("linux_window_identity.rs", "Linux only"),
            ("youtube_identity.rs", "Linux only"),
            ("debug.rs", "opt-in log; portable default is local\\logs"),
            ("commands.rs", "reads only: draw.io install discovery"),
        ];
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
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
                if ALLOWED.iter().any(|(file, _)| relative == *file) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                for (line_number, line) in production_lines(&source) {
                    let code = line.split("//").next().unwrap_or("");
                    if let Some(lookup) = LOOKUPS.iter().find(|lookup| code.contains(*lookup)) {
                        offenders.push(format!("{relative}:{line_number}: {lookup}"));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "GH #719: resolve app state through app.path().app_*_dir() or \
             app_identity::current_app_data_dir(), so a portable instance keeps it \
             beside the exe; or list the file here with the reason it cannot write \
             the host:\n{}",
            offenders.join("\n")
        );
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
