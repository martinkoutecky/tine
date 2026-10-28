//! Settings → Graph → Git and the app's hooks for autocommit and sync (ADR
//! 0078 slices 1 and 2). Git features exist on Linux, Windows and macOS; on
//! iOS and Android Settings says "not available on this device yet" (slice
//! 3), and every hook is a no-op. Both lists name their targets explicitly (AGENTS
//! §2), pinned by `git_targets_are_named`.

use std::path::Path;

use crate::state::GraphSlot;
use std::sync::Arc;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
mod imp {
    use super::*;
    use crate::owner::git_autocommit::{self as autocommit, GitSettings, GitStatus, Policy};
    use crate::owner::git_sync::credentials::Tokens;
    use crate::owner::git_sync_cycle::SyncStatus;

    /// This device's git tokens (app data; `None` in unit tests, which have
    /// no app-data folder).
    fn tokens(app: &tauri::AppHandle) -> Option<Tokens> {
        crate::graph::checkpoint_app_data(app).map(Tokens::new)
    }

    fn read(app: &tauri::AppHandle, root: &Path) -> GitSettings {
        crate::settings::settings_path(app)
            .and_then(|p| crate::settings::read_settings_value_at(&p))
            .map(|json| autocommit::settings_from_json(&json, root))
            .unwrap_or_default()
    }

    /// At bind: start the graph's autocommit worker with its settings.
    pub(crate) fn bound(app: &tauri::AppHandle, slot: &Arc<GraphSlot>) {
        let settings = read(app, &slot.root_key);
        let credentials: Arc<dyn crate::owner::git_sync::Credentials> = match tokens(app) {
            Some(tokens) => Arc::new(tokens),
            None => Arc::new(crate::owner::git_sync::NoCredentials),
        };
        autocommit::start_worker(slot, settings, credentials);
        prepare_ignore(app, slot, settings);
    }

    fn prepare_ignore(app: &tauri::AppHandle, slot: &GraphSlot, settings: GitSettings) {
        if !settings.policy.commits() {
            return;
        }
        let result = crate::graph::checkpoint_app_data(app)
            .ok_or_else(|| "no app-data directory for remembering the attempt".to_owned())
            .and_then(|dir| {
                autocommit::prepare_syncthing_ignore(&slot.store, &slot.root_key, &dir)
            });
        autocommit::report_ignore_setup(&slot.root_key, result);
    }

    fn status(app: &tauri::AppHandle, root: &Path, settings: GitSettings) -> GitStatus {
        let mut status = autocommit::git_status(root, settings);
        let origin = status.remote.as_ref().and_then(|r| r.https_origin.clone());
        status.signed_in = origin.and_then(|o| tokens(app)?.signed_in(&o));
        status
    }

    pub(crate) fn autocommit_status(app: &tauri::AppHandle, slot: &GraphSlot) -> GitStatus {
        status(app, &slot.root_key, read(app, &slot.root_key))
    }

    /// The sync problem the app shows outside Settings: only one that
    /// persists, and only while a sync option is chosen. Reads the worker's
    /// state, never the repository, so every window may poll it.
    pub(crate) fn sync_notice(app: &tauri::AppHandle, slot: &GraphSlot) -> Option<SyncStatus> {
        if !read(app, &slot.root_key).policy.syncs() {
            return None;
        }
        Some(autocommit::last_attempt(&slot.root_key).sync).filter(|s| s.persistent)
    }

    /// "Sync now": commit pending changes, then one sync cycle, on the graph's
    /// git thread. Answers at once; the status shows the cycle running.
    pub(crate) fn sync_now(app: &tauri::AppHandle, slot: &GraphSlot) -> Result<GitStatus, String> {
        let settings = read(app, &slot.root_key);
        if !settings.policy.syncs() {
            return Err("Choose a sync option first.".into());
        }
        if !autocommit::request_sync(&slot.root_key) {
            return Err("The graph's git worker is not running.".into());
        }
        Ok(status(app, &slot.root_key, settings))
    }

    pub(crate) fn disable_line_endings(
        app: &tauri::AppHandle,
        slot: &GraphSlot,
    ) -> Result<GitStatus, String> {
        slot.store
            .check_graph_root()
            .map_err(|e| format!("{e:?}"))?;
        crate::owner::git_sync::line_endings::disable(&slot.root_key)?;
        let settings = read(app, &slot.root_key);
        if settings.policy.syncs() {
            autocommit::request_sync(&slot.root_key);
        }
        Ok(status(app, &slot.root_key, settings))
    }

    /// The HTTPS host the bound graph's remote is on, for a sign-in.
    fn https_origin(slot: &GraphSlot) -> Result<String, String> {
        match crate::owner::git_sync::remote(&slot.root_key) {
            Ok(remote) => remote
                .https_origin
                .ok_or_else(|| "This remote needs no sign-in.".to_owned()),
            Err(_) => Err("Tine can't sync this graph, so it has nothing to sign in to.".into()),
        }
    }

    pub(crate) fn sign_in(
        app: &tauri::AppHandle,
        slot: &GraphSlot,
        username: &str,
        token: &str,
    ) -> Result<GitStatus, String> {
        let tokens = tokens(app).ok_or("Tine has no app data folder")?;
        tokens.sign_in(&https_origin(slot)?, username, token)?;
        // A sync that waited for the sign-in runs now rather than after its back-off.
        if read(app, &slot.root_key).policy.syncs() {
            autocommit::request_sync(&slot.root_key);
        }
        Ok(autocommit_status(app, slot))
    }

    pub(crate) fn sign_out(app: &tauri::AppHandle, slot: &GraphSlot) -> Result<GitStatus, String> {
        let tokens = tokens(app).ok_or("Tine has no app data folder")?;
        tokens.sign_out(&https_origin(slot)?)?;
        Ok(autocommit_status(app, slot))
    }

    pub(crate) fn set_autocommit(
        app: &tauri::AppHandle,
        slot: &GraphSlot,
        policy: Policy,
        interval_minutes: u32,
    ) -> Result<GitStatus, String> {
        let settings = GitSettings {
            policy,
            interval: std::time::Duration::from_secs(
                u64::from(autocommit::clamp_interval(interval_minutes)) * 60,
            ),
        };
        if policy.syncs() {
            // Refused with the repository's reason (ADR §1); the device keeps
            // its previous choice.
            let now = autocommit::git_status(&slot.root_key, settings);
            if let Some((_, why)) = now.sync_unusable {
                return Err(format!("Tine can't sync here: {why}"));
            }
        }
        crate::settings::update_settings(app, |json| {
            autocommit::settings_into_json(json, &slot.root_key, settings)
        })?;
        autocommit::set_settings(&slot.root_key, settings);
        prepare_ignore(app, slot, settings);
        Ok(status(app, &slot.root_key, settings))
    }

    pub(crate) fn around<T, E>(
        root: &Path,
        verb: impl FnOnce() -> String,
        op: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        autocommit::around(root, verb, op)
    }

    /// App exit: one bounded autocommit per graph (G9).
    pub(crate) fn exiting() {
        autocommit::flush_all(std::time::Duration::from_secs(5));
    }

    pub(crate) type Status = GitStatus;
    pub(crate) type PolicyArg = Policy;
    pub(crate) type Notice = SyncStatus;
}

#[cfg(any(target_os = "ios", target_os = "android"))]
mod imp {
    use super::*;

    use crate::owner::git_sync::UNAVAILABLE_REASON;

    #[derive(Debug, Clone, serde::Serialize)]
    pub(crate) struct Status {
        available: bool,
    }

    #[derive(Debug, Clone, Copy, serde::Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub(crate) enum PolicyArg {
        Off,
        Commit,
        SyncOnDemand,
        SyncAutomatically,
    }

    /// Never sent: no worker runs here.
    pub(crate) type Notice = Status;

    pub(crate) fn bound(_app: &tauri::AppHandle, _slot: &Arc<GraphSlot>) {}

    pub(crate) fn autocommit_status(_app: &tauri::AppHandle, _slot: &GraphSlot) -> Status {
        Status { available: false }
    }

    pub(crate) fn set_autocommit(
        _app: &tauri::AppHandle,
        _slot: &GraphSlot,
        _policy: PolicyArg,
        _interval_minutes: u32,
    ) -> Result<Status, String> {
        Err(UNAVAILABLE_REASON.into())
    }

    pub(crate) fn sync_notice(_app: &tauri::AppHandle, _slot: &GraphSlot) -> Option<Notice> {
        None
    }

    pub(crate) fn sync_now(_app: &tauri::AppHandle, _slot: &GraphSlot) -> Result<Status, String> {
        Err(UNAVAILABLE_REASON.into())
    }

    pub(crate) fn disable_line_endings(
        _app: &tauri::AppHandle,
        _slot: &GraphSlot,
    ) -> Result<Status, String> {
        Err(UNAVAILABLE_REASON.into())
    }

    pub(crate) fn sign_in(
        _app: &tauri::AppHandle,
        _slot: &GraphSlot,
        _username: &str,
        _token: &str,
    ) -> Result<Status, String> {
        Err(UNAVAILABLE_REASON.into())
    }

    pub(crate) fn sign_out(_app: &tauri::AppHandle, _slot: &GraphSlot) -> Result<Status, String> {
        Err(UNAVAILABLE_REASON.into())
    }

    pub(crate) fn around<T, E>(
        _root: &Path,
        _verb: impl FnOnce() -> String,
        op: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        op()
    }

    pub(crate) fn exiting() {}
}

pub(crate) use imp::{around, bound, exiting};

#[tauri::command]
pub(crate) async fn git_autocommit_status(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<imp::Status, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || Ok(imp::autocommit_status(&app, &slot))).await
}

#[tauri::command]
pub(crate) async fn set_git_autocommit(
    policy: imp::PolicyArg,
    interval_minutes: u32,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<imp::Status, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || imp::set_autocommit(&app, &slot, policy, interval_minutes)).await
}

#[tauri::command]
pub(crate) async fn git_sync_now(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<imp::Status, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || imp::sync_now(&app, &slot)).await
}

#[tauri::command]
pub(crate) async fn git_disable_line_endings(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<imp::Status, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || imp::disable_line_endings(&app, &slot)).await
}

#[tauri::command]
pub(crate) async fn git_sync_notice(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<Option<imp::Notice>, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || Ok(imp::sync_notice(&app, &slot))).await
}

/// Keep a token for the bound graph's HTTPS remote host. The token goes
/// straight to the private credentials file; no answer or error carries it.
#[tauri::command]
pub(crate) async fn git_sign_in(
    username: String,
    token: String,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<imp::Status, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || imp::sign_in(&app, &slot, &username, &token)).await
}

#[tauri::command]
pub(crate) async fn git_sign_out(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<imp::Status, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || imp::sign_out(&app, &slot)).await
}

#[cfg(test)]
mod tests {
    /// AGENTS §2: every shipped target is named in exactly one list, so a
    /// target never silently compiles into neither.
    #[test]
    fn git_targets_are_named() {
        let source = include_str!("git_settings.rs");
        let desktop =
            r#"#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]"#;
        let mobile = r#"#[cfg(any(target_os = "ios", target_os = "android"))]"#;
        assert_eq!(
            source.matches(desktop).count(),
            2,
            "one desktop module (and this test)"
        );
        assert_eq!(
            source.matches(mobile).count(),
            2,
            "one mobile module (and this test)"
        );
        let expected = cfg!(any(
            target_os = "linux",
            target_os = "windows",
            target_os = "macos"
        ));
        assert_eq!(crate::owner::git_sync::AVAILABLE, expected);
    }
}
