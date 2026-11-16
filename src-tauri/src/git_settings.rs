//! Settings → Graph → Git and the app's hooks for autocommit and sync (ADR
//! 0078 slices 1–3). One implementation for all five shipped targets: no
//! platform `cfg` here, pinned by `git_has_no_platform_arm`.

use std::path::Path;

use crate::state::GraphSlot;
use std::sync::Arc;

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
    let app_data = crate::graph::checkpoint_app_data(app);
    let result = autocommit::prepare_git_ignore(&slot.store, &slot.root_key, app_data.as_deref());
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
    notice_for(read(app, &slot.root_key).policy, &slot.root_key)
}

/// [`sync_notice`] for a policy. A stale `index.lock` Tine offers to remove
/// (row `stale-lock`) shows under every committing policy, since commits
/// wait on it too, and before any sync row (a `busy` row has the same cause).
pub(crate) fn notice_for(policy: Policy, root: &Path) -> Option<SyncStatus> {
    let last = autocommit::last_attempt(root);
    if policy.commits() {
        if let Some(offer) = last.stale_lock_offer {
            return Some(SyncStatus {
                code: Some("stale-lock".into()),
                message: None,
                path: Some(offer.path.clone()),
                persistent: true,
                line_endings: None,
                stale_lock: Some(offer),
                ..last.sync
            });
        }
    }
    if !policy.syncs() {
        return None;
    }
    Some(last.sync).filter(|s| s.persistent)
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

/// "Remove the stale lock" (GS-LOCK): remove the `index.lock` offered as
/// `token` after rechecking it ([`crate::owner::git_stale_lock::remove`]),
/// then let the worker commit (and sync) at once.
pub(crate) fn remove_stale_lock(
    app: &tauri::AppHandle,
    slot: &GraphSlot,
    token: &str,
) -> Result<GitStatus, String> {
    slot.store
        .check_graph_root()
        .map_err(|e| format!("{e:?}"))?;
    crate::owner::git_stale_lock::remove(&slot.root_key, token).map_err(|why| why.message())?;
    let settings = read(app, &slot.root_key);
    autocommit::lock_removed(&slot.root_key, settings.policy.syncs());
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
    if policy.commits() && crate::owner::git_sync::initial_push_pending(&slot.root_key) {
        return Err("Finish repository setup before enabling automatic commits or sync.".into());
    }
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

#[tauri::command]
pub(crate) async fn git_autocommit_status(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || Ok(autocommit_status(&app, &slot))).await
}

#[tauri::command]
pub(crate) async fn set_git_autocommit(
    policy: Policy,
    interval_minutes: u32,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || set_autocommit(&app, &slot, policy, interval_minutes)).await
}

#[tauri::command]
pub(crate) async fn git_sync_now(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || sync_now(&app, &slot)).await
}

#[tauri::command]
pub(crate) async fn git_disable_line_endings(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || disable_line_endings(&app, &slot)).await
}

#[tauri::command]
pub(crate) async fn git_remove_stale_lock(
    token: String,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || remove_stale_lock(&app, &slot, &token)).await
}

#[tauri::command]
pub(crate) async fn git_sync_notice(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<Option<SyncStatus>, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || Ok(sync_notice(&app, &slot))).await
}

/// Keep a token for the bound graph's HTTPS remote host. The token goes
/// straight to the private credentials file; no answer or error carries it.
#[tauri::command]
pub(crate) async fn git_sign_in(
    username: String,
    token: String,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || sign_in(&app, &slot, &username, &token)).await
}

/// The app went to the background (`foreground: false`) or came back (ADR
/// 0078 §3, slice 3). The frontend sends it on Android and iOS, after its
/// own background flush, as the app's visibility changes (`gitLifecycle.ts`);
/// desktop keeps committing on the cadence and at quit. Answers at once: the
/// graphs' git threads do the work.
#[tauri::command]
pub(crate) async fn git_app_lifecycle(foreground: bool) {
    if foreground {
        autocommit::foregrounded();
    } else {
        autocommit::backgrounded();
    }
}

#[tauri::command]
pub(crate) async fn git_sign_out(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || sign_out(&app, &slot)).await
}

#[cfg(test)]
mod tests {
    /// AGENTS §2 / ADR 0078 slice 3: git is one implementation on all five
    /// shipped targets. A platform arm here, in the seen commands or in the
    /// owner's module list would bring back a target compiled without git.
    #[test]
    fn git_has_no_platform_arm() {
        for (file, source) in [
            ("git_settings.rs", include_str!("git_settings.rs")),
            ("seen.rs", include_str!("seen.rs")),
        ] {
            let code = source.split("#[cfg(test)]").next().unwrap();
            assert!(!code.contains("target_os"), "{file} has a platform arm");
            assert!(!code.contains("cfg(desktop)"), "{file} has a platform arm");
            assert!(!code.contains("cfg(mobile)"), "{file} has a platform arm");
        }
        let modules = include_str!("owner/mod.rs");
        for module in [
            "git_autocommit",
            "git_identity",
            "git_pull",
            "git_run",
            "git_stale_lock",
            "git_sync",
            "git_sync_cycle",
            "seen",
        ] {
            let line = format!("pub(crate) mod {module};");
            let at = modules.find(&line).unwrap_or_else(|| panic!("{line}"));
            let previous = modules[..at].lines().next_back().unwrap_or("");
            assert!(
                !previous.trim_start().starts_with("#["),
                "owner::{module} is gated `{previous}`"
            );
        }
    }
}
