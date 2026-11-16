//! Semantic status/recovery for the app's editable owner fallback. No raw
//! endpoint or CLI verb is exposed to the webview. D-20261027-OF.
use std::sync::Arc;

use serde::Serialize;
use tauri::Manager;

use crate::owner::{namespace, OwnerError};
use crate::state::GraphSlot;

#[derive(Serialize)]
pub(crate) struct ProtectionIssue {
    reason: &'static str,
    details: String,
    repairable: bool,
    instructions: String,
    command: String,
}

fn issue(error: &OwnerError) -> ProtectionIssue {
    let dir = namespace::directory().ok();
    let path = dir
        .as_deref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let repairable = namespace::repair_available();
    let reason = match error {
        OwnerError::Unsafe(_) if repairable => "permissions",
        OwnerError::Unsafe(_) => "namespace",
        OwnerError::Lock(_) => "lock",
        _ => "handover",
    };
    let (instructions, command) = recovery_instructions(&path, repairable);
    ProtectionIssue {
        reason,
        details: format!(
            "{}: {}\ncoordination directory: {}\nTine {} ({})",
            error.code(),
            error,
            path,
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        ),
        repairable,
        instructions,
        command,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn quote_shell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn recovery_instructions(path: &str, repairable: bool) -> (String, String) {
    if repairable {
        return (
            "permissions".into(),
            format!("chmod 700 -- {}", quote_shell(path)),
        );
    }
    #[cfg(target_os = "linux")]
    if let Ok(id) = std::env::var("FLATPAK_ID") {
        // Only instruct; never execute user/platform configuration commands.
        return (
            "flatpak".into(),
            format!(
                "flatpak override --user --filesystem={} {}",
                quote_shell(path),
                quote_shell(&id)
            ),
        );
    }
    // Do not suggest deleting/moving owners or a lock: a live holder must keep
    // the same inode. Stop all hosts before an administrator changes access.
    ("access".into(), format!("ls -ld -- {}", quote_shell(path)))
}

#[cfg(target_os = "windows")]
fn recovery_instructions(path: &str, _: bool) -> (String, String) {
    // Windows security owns DACL policy; never silently rewrite it here.
    (
        "windows-access".into(),
        format!("explorer.exe /select,\"{}\"", path),
    )
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn recovery_instructions(_: &str, _: bool) -> (String, String) {
    (String::new(), String::new())
}

fn current(slot: &GraphSlot) -> Option<ProtectionIssue> {
    if slot.owner.lock().unwrap().is_some() {
        return None;
    }
    slot.owner_failure.lock().unwrap().as_ref().map(issue)
}

#[tauri::command]
pub(crate) async fn owner_protection_status(
    state: crate::state::GraphContext<'_>,
) -> Result<Option<ProtectionIssue>, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || Ok(current(&slot))).await
}

/// Only one attempt per binding. The slow yield wait holds neither the host nor
/// registry gate. Attachment takes host -> owner and rechecks revocation, so a
/// late claim cannot outlive a retired/adopted host. A timeout leaves it unleased.
#[cfg(desktop)]
fn retry_with(
    slot: &Arc<GraphSlot>,
    attempt: impl FnOnce() -> (
        Result<Option<crate::owner::lease::Claimed>, String>,
        Option<OwnerError>,
    ),
) {
    retry_at_publication(slot, attempt, || {});
}

#[cfg(desktop)]
fn retry_at_publication(
    slot: &Arc<GraphSlot>,
    attempt: impl FnOnce() -> (
        Result<Option<crate::owner::lease::Claimed>, String>,
        Option<OwnerError>,
    ),
    before_owner: impl FnOnce(),
) {
    use std::sync::atomic::Ordering;
    let Ok(_attempt) = slot.owner_retry.try_lock() else {
        return;
    };
    if slot.background_cancelled.load(Ordering::Acquire) || slot.owner.lock().unwrap().is_some() {
        return;
    }
    let (result, failure) = attempt();
    let host = slot.host.read().unwrap();
    if slot.background_cancelled.load(Ordering::Acquire)
        || !matches!(*host, crate::state::PageHostSlot::Running(_))
    {
        return;
    }
    before_owner();
    let mut owner = slot.owner.lock().unwrap();
    // Retirement cancels before taking this same mutex to pause admission.
    // Either it sees our lease and pauses it, or we see its cancellation and
    // release the claim without publishing an admitting endpoint.
    if slot.background_cancelled.load(Ordering::Acquire) || owner.is_some() {
        return;
    }
    match result {
        Ok(Some(claimed)) => {
            *owner = Some(claimed.serve_app(slot));
            *slot.owner_failure.lock().unwrap() = None;
        }
        Ok(None) => *slot.owner_failure.lock().unwrap() = failure,
        Err(message) => *slot.owner_failure.lock().unwrap() = Some(OwnerError::Start(message)),
    }
}

#[cfg(desktop)]
fn retry(app: &tauri::AppHandle, slot: &Arc<GraphSlot>, repair: bool) {
    retry_with(slot, || {
        if repair {
            if let Err(error) = namespace::repair() {
                return (Ok(None), Some(error));
            }
        }
        let data = match app.path().app_data_dir() {
            Ok(data) => data,
            Err(error) => return (Err(error.to_string()), None),
        };
        let mut failure = None;
        let result = crate::owner::lease::claim_for_app(
            &slot.root_key,
            crate::owner::wire::Drafts {
                app_data: data.display().to_string(),
                graph_id: crate::settings::host_graph_id(&slot.root_key),
            },
            std::time::Duration::from_secs(60),
            |_| {},
            |error| failure = Some(error.clone()),
        );
        (result, failure)
    });
}

#[tauri::command]
pub(crate) async fn retry_owner_protection(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
    repair: bool,
) -> Result<Option<ProtectionIssue>, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || {
        #[cfg(desktop)]
        retry(&app, &slot, repair);
        #[cfg(any(target_os = "ios", target_os = "android"))]
        let _ = (app, repair);
        Ok(current(&slot))
    })
    .await
}

pub(crate) fn start_retry(app: &tauri::AppHandle, slot: &Arc<GraphSlot>) {
    #[cfg(desktop)]
    {
        if slot.owner.lock().unwrap().is_some() {
            return;
        }
        // Adoption can inherit an unleased host. Its new binding must also
        // retry and show a warning, even though the old diagnosis retired.
        if slot.owner_failure.lock().unwrap().is_none() {
            *slot.owner_failure.lock().unwrap() = Some(OwnerError::Start(
                "the graph has no owner lease; retrying".into(),
            ));
        }
        let weak = Arc::downgrade(slot);
        let app = app.clone();
        let _ = std::thread::Builder::new()
            .name("tine-owner-retry".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let Some(slot) = weak.upgrade() else {
                    return;
                };
                if slot
                    .background_cancelled
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    return;
                }
                retry(&app, &slot, false);
                if slot.owner.lock().unwrap().is_some() {
                    return;
                }
            });
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let _ = (app, slot);
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;
    use crate::owner::{lease::Claimed, lock::OwnerLock, namespace::Namespace};
    use crate::state::PageHostSlot;

    #[test]
    fn retirement_between_cancellation_check_and_owner_lock_prevents_publication() {
        use std::sync::Barrier;
        let data = tempfile::tempdir().unwrap();
        let store = Arc::new(
            tine_store::Store::open(data.path(), Default::default())
                .unwrap()
                .0,
        );
        let slot = Arc::new(GraphSlot::new(store, data.path().to_owned()));
        *slot.host.write().unwrap() = PageHostSlot::Running(
            tine_store::PageHost::start_for_tests(&slot.store, data.path()).unwrap(),
        );
        let paths = Namespace::at(data.path().join("owners"))
            .unwrap()
            .graph(&slot.root_key)
            .unwrap();
        let checked = Arc::new(Barrier::new(2));
        let publish = Arc::new(Barrier::new(2));
        let worker = {
            let (slot, paths, checked, publish) = (
                slot.clone(),
                paths.clone(),
                checked.clone(),
                publish.clone(),
            );
            std::thread::spawn(move || {
                retry_at_publication(
                    &slot,
                    || {
                        let lock = OwnerLock::try_take(&paths.lock, "app").unwrap().unwrap();
                        (Ok(Some(Claimed::from_lock(lock, paths))), None)
                    },
                    || {
                        checked.wait();
                        publish.wait();
                    },
                )
            })
        };
        checked.wait();
        // Keep the host alive while actual retirement observes no lease and
        // skips pausing admission. The retry already passed its first check.
        let host = slot.host.read().unwrap();
        slot.cancel_background();
        assert!(slot.owner.lock().unwrap().is_none());
        let retirement = crate::host_retirement::HostRetirement::default();
        assert!(retirement.retire(slot.clone()).is_none());
        publish.wait();
        worker.join().unwrap();
        let attached = slot.owner.lock().unwrap().is_some();
        drop(host);
        retirement
            .wait_idle(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(
            !attached,
            "retirement saw no lease: a late endpoint must never admit requests"
        );
        assert!(OwnerLock::try_take(&paths.lock, "serve").unwrap().is_some());
    }

    #[test]
    fn retries_attach_once_and_cancelled_bindings_never_take_a_lease() {
        let data = tempfile::tempdir().unwrap();
        let root = data.path().join("graph");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        let store = Arc::new(
            tine_store::Store::open(&root, Default::default())
                .unwrap()
                .0,
        );
        let slot = Arc::new(GraphSlot::new(store.clone(), root));
        *slot.host.write().unwrap() = PageHostSlot::Running(
            tine_store::PageHost::start_for_tests(&store, data.path()).unwrap(),
        );
        let paths = Namespace::at(data.path().join("owners"))
            .unwrap()
            .graph(&slot.root_key)
            .unwrap();
        *slot.owner_failure.lock().unwrap() = Some(OwnerError::Lock("transient failure".into()));
        retry_with(&slot, || {
            (Ok(None), Some(OwnerError::Lock("still unavailable".into())))
        });
        // Inspect the binding state directly: production diagnostics derive
        // the real account namespace, which tests must never probe.
        assert!(slot
            .owner_failure
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .to_string()
            .contains("still unavailable"));
        retry_with(&slot, || {
            let lock = OwnerLock::try_take(&paths.lock, "app").unwrap().unwrap();
            (Ok(Some(Claimed::from_lock(lock, paths.clone()))), None)
        });
        assert!(current(&slot).is_none());
        assert!(OwnerLock::try_take(&paths.lock, "serve").unwrap().is_none());
        retry_with(&slot, || {
            panic!("already leased bindings do not claim again")
        });
        drop(slot);
        let slot = Arc::new(GraphSlot::new(store, data.path().join("other")));
        slot.cancel_background();
        retry_with(&slot, || panic!("a retired binding must not claim"));
    }

    #[test]
    fn a_claim_finishing_after_revocation_is_released() {
        let data = tempfile::tempdir().unwrap();
        let store = Arc::new(
            tine_store::Store::open(data.path(), Default::default())
                .unwrap()
                .0,
        );
        let slot = Arc::new(GraphSlot::new(store, data.path().to_owned()));
        *slot.host.write().unwrap() = PageHostSlot::Running(
            tine_store::PageHost::start_for_tests(&slot.store, data.path()).unwrap(),
        );
        let paths = Namespace::at(data.path().join("owners"))
            .unwrap()
            .graph(&slot.root_key)
            .unwrap();
        retry_with(&slot, || {
            let lock = OwnerLock::try_take(&paths.lock, "app").unwrap().unwrap();
            slot.cancel_background();
            (Ok(Some(Claimed::from_lock(lock, paths.clone()))), None)
        });
        assert!(slot.owner.lock().unwrap().is_none());
        assert!(OwnerLock::try_take(&paths.lock, "serve").unwrap().is_some());
    }

    #[test]
    fn timeout_and_an_inflight_retry_never_authorize_a_second_claim() {
        let data = tempfile::tempdir().unwrap();
        let store = Arc::new(
            tine_store::Store::open(data.path(), Default::default())
                .unwrap()
                .0,
        );
        let slot = Arc::new(GraphSlot::new(store, data.path().to_owned()));
        *slot.host.write().unwrap() = PageHostSlot::Running(
            tine_store::PageHost::start_for_tests(&slot.store, data.path()).unwrap(),
        );
        let guard = slot.owner_retry.lock().unwrap();
        retry_with(&slot, || {
            panic!("a simultaneous retry cannot start a claim")
        });
        drop(guard);
        retry_with(&slot, || (Err("handover timed out".into()), None));
        assert!(slot.owner.lock().unwrap().is_none());
        assert!(slot
            .owner_failure
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .to_string()
            .contains("handover timed out"));
    }
}
