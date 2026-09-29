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
//! Desktop Linux only: Android/iOS use sandboxed app dirs and no report exists
//! for Windows/macOS. Cost: one create+remove of an empty probe file per
//! launch; nothing persisted.

#[cfg(all(desktop, target_os = "linux"))]
use crate::debug::{diag, diag_private};
#[cfg(all(desktop, target_os = "linux"))]
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The fallback data home iff this launch relocated; taken once by the command.
static RELOCATED_TO: Mutex<Option<String>> = Mutex::new(None);

/// Can files be created under `base` the way Tauri and WebKitGTK are about to?
/// `create_dir_all` alone succeeds on an existing directory nobody may write,
/// which is exactly the reported case, so write and remove a probe file in the
/// app dir when it exists (its own mode matters then) and in the base otherwise.
#[cfg(all(desktop, target_os = "linux"))]
fn probe_writable(base: &Path, identifier: &str) -> std::io::Result<()> {
    let app_dir = base.join(identifier);
    let target = if app_dir.is_dir() {
        app_dir
    } else {
        base.to_path_buf()
    };
    std::fs::create_dir_all(&target)?;
    let probe = target.join(format!(".tine-write-probe-{}", std::process::id()));
    std::fs::write(&probe, b"")?;
    std::fs::remove_file(&probe)?;
    Ok(())
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
            format!("app-data home relocated to {} for this launch", candidate.display()),
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

/// Command: the fallback data home ONCE if this launch relocated, then `None`
/// (so a webview reload does not re-toast). `None` on every ordinary launch.
#[tauri::command]
pub(crate) fn take_data_home_fallback_notice() -> Option<String> {
    RELOCATED_TO
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
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
        probe_writable(&base, "page.tine.TineOG").unwrap();
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
        let error = probe_writable(&base, "page.tine.TineOG").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        set_mode(&base, 0o755);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn an_unwritable_existing_app_dir_is_rejected_under_a_writable_base() {
        let base = tmp("ro-app");
        let app = base.join("page.tine.TineOG");
        std::fs::create_dir_all(&app).unwrap();
        set_mode(&app, 0o555);
        assert!(probe_writable(&base, "page.tine.TineOG").is_err());
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
        assert_eq!(
            take_data_home_fallback_notice(),
            Some("/tmp/example".to_string())
        );
        assert_eq!(take_data_home_fallback_notice(), None);
    }
}
