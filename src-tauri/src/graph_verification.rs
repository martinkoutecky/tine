//! Settings → Help & diagnostics → "Verify synchronized graph" commands: build
//! an exact-byte manifest of the open graph (`tine_graph_features::graph_verification`),
//! cancel it, and save the report the user chose to keep.
//!
//! Read-only over the graph; the hashing runs off the UI thread and stops when
//! the caller cancels or the window's graph binding is replaced. Progress events carry the
//! operation id (the frontend ignores other operations) and are emitted at most
//! every 100 ms.

use crate::state::{slot_for_context, GraphContext};
use serde::Serialize;
use std::collections::{hash_map::Entry, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tine_graph_features::graph_verification::{verify_graph_bytes, Manifest};

static ACTIVE: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn active() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Removes its operation from the registry on every exit, including a panic.
struct Registration(String);

impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut jobs) = active().lock() {
            jobs.remove(&self.0);
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    operation_id: String,
    processed: usize,
    total: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphVerificationReport {
    text: String,
    suggested_file_name: String,
    total_files: usize,
    total_bytes: u64,
    aggregate_digest: Option<String>,
    complete: bool,
}

fn registry_error() -> String {
    "graph verification registry is unavailable".into()
}

/// Hash every graph text file and return the report text. `cancelled` in the
/// error string is the frontend's cue to stay silent.
#[tauri::command]
pub(crate) async fn create_graph_verification(
    state: GraphContext<'_>,
    operation_id: String,
) -> Result<GraphVerificationReport, String> {
    if operation_id.is_empty() || operation_id.len() > 128 {
        return Err("invalid graph verification operation id".into());
    }
    let slot = slot_for_context(&state)?;
    let window = state.window.clone();
    drop(state);
    let cancelled = Arc::new(AtomicBool::new(false));
    match active()
        .lock()
        .map_err(|_| registry_error())?
        .entry(operation_id.clone())
    {
        Entry::Vacant(entry) => {
            entry.insert(Arc::clone(&cancelled));
        }
        Entry::Occupied(_) => {
            return Err("graph verification operation id is already active".into())
        }
    }
    let registration = Registration(operation_id.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let _registration = registration;
        let stop = || {
            cancelled.load(Ordering::Acquire) || slot.background_cancelled.load(Ordering::Acquire)
        };
        let mut last = Instant::now() - Duration::from_secs(1);
        let manifest = verify_graph_bytes(&slot.store, &stop, &mut |processed, total| {
            if processed != total && last.elapsed() < Duration::from_millis(100) {
                return;
            }
            last = Instant::now();
            let _ = window.emit(
                "graph-verification-progress",
                Progress {
                    operation_id: operation_id.clone(),
                    processed,
                    total,
                },
            );
        })
        .map_err(|_| "graph verification cancelled".to_owned())?;
        let text = manifest
            .to_report()
            .map_err(|error| format!("graph verification report could not be encoded: {error}"))?;
        Ok(GraphVerificationReport {
            text,
            suggested_file_name: format!(
                "tine-graph-verification-{}.json",
                manifest.generated_at_unix_ms
            ),
            total_files: manifest.files.len(),
            total_bytes: manifest.total_bytes(),
            aggregate_digest: manifest.aggregate_digest.clone(),
            complete: manifest.complete,
        })
    })
    .await
    .map_err(|error| format!("graph verification task failed: {error}"))?
}

/// Ask a running verification to stop. Unknown ids are ignored.
#[tauri::command]
pub(crate) fn cancel_graph_verification(operation_id: String) -> Result<(), String> {
    if let Some(flag) = active()
        .lock()
        .map_err(|_| registry_error())?
        .get(&operation_id)
    {
        flag.store(true, Ordering::Release);
    }
    Ok(())
}

/// Save a verification report where the user chooses (desktop save dialog);
/// `false` when the user cancelled. The text must parse as a `tine-graph-bytes`
/// manifest. Mobile has no save dialog: Copy graph report.
#[tauri::command]
pub(crate) async fn save_graph_verification_report(
    app: tauri::AppHandle,
    text: String,
) -> Result<bool, String> {
    let manifest = Manifest::from_report(&text)?;
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt as _;
        let suggested = format!(
            "tine-graph-verification-{}.json",
            manifest.generated_at_unix_ms
        );
        let chosen = tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_file_name(suggested)
                .add_filter("JSON", &["json"])
                .blocking_save_file()
        })
        .await
        .map_err(|_| "The save dialog failed.".to_owned())?;
        let Some(chosen) = chosen else {
            return Ok(false);
        };
        let path = chosen
            .into_path()
            .map_err(|_| "The chosen destination is not a local file.".to_owned())?;
        crate::flight_store::FlightStore::save_report(&path, &text).map_err(|error| {
            crate::debug::diag_private("graph-verification-save-failed", error.to_string());
            "The graph verification report could not be saved.".to_owned()
        })?;
        Ok(true)
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, manifest, text);
        Err("Save report is available on desktop; use Copy graph report on this device.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_flips_only_the_named_operation_and_ignores_unknown_ids() {
        let mine = Arc::new(AtomicBool::new(false));
        let other = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = active().lock().unwrap();
            jobs.insert("gv-test-mine".into(), Arc::clone(&mine));
            jobs.insert("gv-test-other".into(), Arc::clone(&other));
        }
        cancel_graph_verification("gv-test-unknown".into()).unwrap();
        cancel_graph_verification("gv-test-mine".into()).unwrap();
        assert!(mine.load(Ordering::Acquire) && !other.load(Ordering::Acquire));
        drop(Registration("gv-test-mine".into()));
        drop(Registration("gv-test-other".into()));
        assert!(!active().lock().unwrap().contains_key("gv-test-mine"));
    }
}
