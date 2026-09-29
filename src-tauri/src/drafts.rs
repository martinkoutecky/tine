//! Crash-surviving unsaved drafts (og ADR 0061, family 9; GH #540).
//!
//! **Question answered.** [`load_drafts`]: which drafts did an earlier session
//! of this graph keep because they could not be saved?
//! **Operations accepted.** [`store_draft`] replaces one record;
//! [`retire_draft`] removes one.
//!
//! **Layout.** One file per graph, `<app data>/drafts/<graph-id>.v1.json`, never
//! inside the graph: `{"version":1,"drafts":[record…]}`. A record is a JSON
//! object with a unique non-empty `id` and a `kind` of `unsaved` (a page whose
//! save failed or that is in conflict) or `live-conflict` (reserved for the
//! Concord live-draft capsule, og 8e: one store, not two). The frontend owns the
//! rest of the record's fields. Every write is `device_io::atomic_write` (temp +
//! fsync + rename + directory sync).
//!
//! **Bounds.** At most [`MAX_RECORDS`] records and [`MAX_BYTES`] bytes.
//!
//! **Refusals.** A write past a bound is refused and the draft stays in the
//! window (scenario: disk error / exhaustion; the bound keeps a stuck page that
//! is edited for hours from growing the file without limit). An unreadable,
//! malformed or oversized file never blocks opening: it is set aside as
//! `.unreadable-<n>` and the store starts empty (scenario: crash or power loss
//! leaving a torn file, disk error, or a sync client delivering another build's
//! file into app data). The bytes are kept, not deleted.
//!
//! **Cost.** Records are written only while a page is at risk, never on an
//! ordinary save. One write is O(file bytes) ≤ 8 MiB, one file.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::Manager;

const VERSION: u64 = 1;
pub(crate) const MAX_RECORDS: usize = 64;
pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
static DRAFTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Deserialize, Serialize)]
struct Envelope {
    version: u64,
    drafts: Vec<Value>,
}

fn record_id(record: &Value) -> Result<&str, String> {
    match record.get("kind").and_then(Value::as_str) {
        Some("unsaved" | "live-conflict") => {}
        _ => return Err("a draft record needs kind unsaved or live-conflict".into()),
    }
    record
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "a draft record needs an id".into())
}

fn decode(bytes: &[u8]) -> Result<Vec<Value>, String> {
    if bytes.len() > MAX_BYTES {
        return Err("draft store is larger than its bound".into());
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if envelope.version != VERSION {
        return Err(format!(
            "unsupported draft store version {}",
            envelope.version
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for record in &envelope.drafts {
        if !ids.insert(record_id(record)?.to_owned()) {
            return Err("duplicate draft record".into());
        }
    }
    Ok(envelope.drafts)
}

/// Set an unreadable file aside and start empty (see the module refusals).
fn set_aside(path: &Path) -> Result<(), String> {
    for n in 0u32.. {
        let aside = path.with_extension(format!("json.unreadable-{n}"));
        if aside.exists() {
            continue;
        }
        crate::device_io::move_file_noreplace(path, &aside).map_err(|e| e.to_string())?;
        return Ok(());
    }
    unreachable!("u32 range exhausted")
}

fn load_unlocked(path: &Path) -> Result<Vec<Value>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    match decode(&bytes) {
        Ok(drafts) => Ok(drafts),
        Err(_) => {
            set_aside(path)?;
            Ok(Vec::new())
        }
    }
}

fn write_unlocked(path: &Path, drafts: Vec<Value>) -> Result<(), String> {
    if drafts.is_empty() {
        return match std::fs::remove_file(path) {
            Ok(()) => path
                .parent()
                .map_or(
                    Ok(()),
                    tine_store::directory_durability::sync_directory_entry,
                )
                .map_err(|e| e.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    if drafts.len() > MAX_RECORDS {
        return Err(format!("the draft store keeps at most {MAX_RECORDS} pages"));
    }
    let bytes = serde_json::to_vec(&Envelope {
        version: VERSION,
        drafts,
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("the draft store is full".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::device_io::atomic_write(path, &bytes).map_err(|e| e.to_string())
}

pub(crate) fn load_at(path: &Path) -> Result<Vec<Value>, String> {
    let _guard = DRAFTS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    load_unlocked(path)
}

pub(crate) fn store_at(path: &Path, record: Value) -> Result<(), String> {
    let id = record_id(&record)?.to_owned();
    let _guard = DRAFTS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut drafts = load_unlocked(path)?;
    match drafts
        .iter_mut()
        .find(|r| record_id(r).ok() == Some(id.as_str()))
    {
        Some(existing) => *existing = record,
        None => drafts.push(record),
    }
    write_unlocked(path, drafts)
}

pub(crate) fn retire_at(path: &Path, id: &str) -> Result<(), String> {
    let _guard = DRAFTS_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut drafts = load_unlocked(path)?;
    let before = drafts.len();
    drafts.retain(|r| record_id(r).ok() != Some(id));
    if drafts.len() == before {
        return Ok(());
    }
    write_unlocked(path, drafts)
}

fn drafts_path(
    app: &tauri::AppHandle,
    state: &crate::state::GraphContext<'_>,
) -> Result<PathBuf, String> {
    let slot = crate::state::slot_for_context(state)?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let id = crate::settings::session_id(&slot.root_key);
    let stem = id.strip_suffix(".json").unwrap_or(&id);
    Ok(dir.join("drafts").join(format!("{stem}.v1.json")))
}

#[tauri::command]
pub(crate) fn load_drafts(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<Vec<Value>, String> {
    load_at(&drafts_path(&app, &state)?)
}

#[tauri::command]
pub(crate) fn store_draft(
    record: Value,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<(), String> {
    store_at(&drafts_path(&app, &state)?, record)
}

#[tauri::command]
pub(crate) fn retire_draft(
    id: String,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<(), String> {
    retire_at(&drafts_path(&app, &state)?, &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn record(id: &str, text: &str) -> Value {
        json!({ "id": id, "kind": "unsaved", "page": { "name": id, "blocks": [{ "raw": text }] } })
    }

    #[test]
    fn records_survive_a_reopen_and_the_last_retirement_removes_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("drafts/g.v1.json");
        store_at(&path, record("s1:P", "first")).unwrap();
        store_at(&path, record("s1:Q", "other")).unwrap();
        store_at(&path, record("s1:P", "second")).unwrap();
        let loaded = load_at(&path).unwrap();
        assert_eq!(
            loaded,
            vec![record("s1:P", "second"), record("s1:Q", "other")]
        );
        retire_at(&path, "s1:P").unwrap();
        retire_at(&path, "s1:missing").unwrap();
        assert_eq!(load_at(&path).unwrap(), vec![record("s1:Q", "other")]);
        retire_at(&path, "s1:Q").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn a_torn_or_foreign_file_is_set_aside_and_never_blocks_open() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("g.v1.json");
        for bytes in [
            &b"{\"version\":1,\"drafts\":[{\"id\":\"s1:P\",\"kind\":\"uns"[..],
            &b"{\"version\":9,\"drafts\":[]}"[..],
            &b"{\"version\":1,\"drafts\":[{\"id\":\"a\",\"kind\":\"other\"}]}"[..],
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(load_at(&path).unwrap().is_empty());
            assert!(!path.exists());
        }
        let aside: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(aside.len(), 3, "each unreadable file is kept: {aside:?}");
        store_at(&path, record("s2:P", "after")).unwrap();
        assert_eq!(load_at(&path).unwrap(), vec![record("s2:P", "after")]);
    }

    #[test]
    fn writes_past_a_bound_are_refused_and_keep_the_earlier_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("g.v1.json");
        for i in 0..MAX_RECORDS {
            store_at(&path, record(&format!("s:{i}"), "x")).unwrap();
        }
        assert!(store_at(&path, record("s:overflow", "x")).is_err());
        assert!(store_at(&path, record("s:0", &"y".repeat(MAX_BYTES))).is_err());
        assert_eq!(load_at(&path).unwrap().len(), MAX_RECORDS);
        assert_eq!(load_at(&path).unwrap()[0], record("s:0", "x"));
    }

    #[test]
    fn a_record_without_an_id_or_known_kind_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("g.v1.json");
        assert!(store_at(&path, json!({ "kind": "unsaved" })).is_err());
        assert!(store_at(&path, json!({ "id": "a", "kind": "other" })).is_err());
        assert!(!path.exists());
    }
}
