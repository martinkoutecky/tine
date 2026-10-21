//! Settings → Graph → Git and the app's hooks for autocommit (ADR 0078
//! slice 1). Git features exist on Linux, Windows and macOS; on iOS and
//! Android Settings says "not available on this device yet" (slice 3), and
//! every hook is a no-op. Both lists name their targets explicitly (AGENTS
//! §2), pinned by `git_targets_are_named`.

use std::path::Path;

use crate::state::GraphSlot;
use std::sync::Arc;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
mod imp {
    use super::*;
    use crate::owner::git_autocommit::{self as autocommit, GitSettings, GitStatus, Policy};

    pub(crate) const AVAILABLE: bool = true;

    fn read(app: &tauri::AppHandle, root: &Path) -> GitSettings {
        crate::settings::settings_path(app)
            .and_then(|p| crate::settings::read_settings_value_at(&p))
            .map(|json| autocommit::settings_from_json(&json, root))
            .unwrap_or_default()
    }

    /// At bind: start the graph's autocommit worker with its settings.
    pub(crate) fn bound(app: &tauri::AppHandle, slot: &Arc<GraphSlot>) {
        autocommit::start(slot, read(app, &slot.root_key));
    }

    pub(crate) fn status(app: &tauri::AppHandle, slot: &GraphSlot) -> GitStatus {
        autocommit::status(&slot.root_key, read(app, &slot.root_key))
    }

    pub(crate) fn set(
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
        crate::settings::update_settings(app, |json| {
            autocommit::settings_into_json(json, &slot.root_key, settings)
        })?;
        autocommit::set_settings(&slot.root_key, settings);
        Ok(autocommit::status(&slot.root_key, settings))
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
}

#[cfg(any(target_os = "ios", target_os = "android"))]
mod imp {
    use super::*;

    pub(crate) const AVAILABLE: bool = false;

    #[derive(Debug, Clone, serde::Serialize)]
    pub(crate) struct Status {
        available: bool,
    }

    #[derive(Debug, Clone, Copy, serde::Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub(crate) enum PolicyArg {
        Off,
        Commit,
    }

    pub(crate) fn bound(_app: &tauri::AppHandle, _slot: &Arc<GraphSlot>) {}

    pub(crate) fn status(_app: &tauri::AppHandle, _slot: &GraphSlot) -> Status {
        Status { available: false }
    }

    pub(crate) fn set(
        _app: &tauri::AppHandle,
        _slot: &GraphSlot,
        _policy: PolicyArg,
        _interval_minutes: u32,
    ) -> Result<Status, String> {
        Err("git is not available on this device yet".into())
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
    crate::state::off_ui(move || Ok(imp::status(&app, &slot))).await
}

#[tauri::command]
pub(crate) async fn set_git_autocommit(
    policy: imp::PolicyArg,
    interval_minutes: u32,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<imp::Status, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || imp::set(&app, &slot, policy, interval_minutes)).await
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
        assert_eq!(super::imp::AVAILABLE, expected);
    }
}
