//! In-app CSS editing: fixed paths, bounded reads and existing guarded writers.
use crate::state::{slot_for_context, GraphContext};
use std::{io, path::Path, sync::Mutex};
use tauri::{Emitter, Manager};
use tine_store::{Area, Content, FileRev, Store, StoreError, TxOutcome};

const MAX_CSS: u64 = tine_store::PARSE_INPUT_MAX_BYTES;
static GLOBAL_CSS_LOCK: Mutex<()> = Mutex::new(());

#[derive(serde::Serialize)]
pub(crate) struct CssFile {
    text: String,
    revision: Option<String>,
}

fn graph_read(store: &Store) -> Result<CssFile, String> {
    let id = store
        .file_id(Area::Meta, "custom.css")
        .map_err(|e| format!("{e:?}"))?;
    match store.read(&id, Some(MAX_CSS)) {
        Ok((bytes, rev)) => Ok(CssFile {
            text: String::from_utf8(bytes).map_err(|e| e.to_string())?,
            revision: Some(rev.into()),
        }),
        Err(StoreError::NotFound) => Ok(CssFile {
            text: String::new(),
            revision: None,
        }),
        Err(error) => Err(format!("{error:?}")),
    }
}

fn graph_save(store: &Store, text: String, revision: Option<String>) -> Result<String, String> {
    if text.len() as u64 > MAX_CSS {
        return Err("stylesheet is too large".into());
    }
    let id = store
        .file_id(Area::Meta, "custom.css")
        .map_err(|e| format!("{e:?}"))?;
    let next = String::from(FileRev::from_bytes(text.as_bytes()));
    let mut tx = store.transaction(None);
    if let Some(rev) = revision {
        tx.replace(&id, rev.into(), text.into_bytes());
    } else {
        tx.create(&id, Content::Bytes(text.into_bytes()));
    }
    match tx.commit() {
        TxOutcome::Committed { .. } => Ok(next),
        other => {
            crate::debug::diag_private("css-save-refused", format!("{other:?}"));
            Err("CSS save refused: the file changed or could not be written. Reload the disk version before retrying.".into())
        }
    }
}

fn global_read(path: &Path) -> Result<CssFile, String> {
    match crate::device_io::read_bounded(path, 32 * 1024 * 1024) {
        Ok(Some(bytes)) => Ok(CssFile {
            revision: Some(FileRev::from_bytes(&bytes).into()),
            text: String::from_utf8(bytes).map_err(|e| e.to_string())?,
        }),
        Ok(None) => Err("stylesheet is too large".into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(CssFile {
            text: String::new(),
            revision: None,
        }),
        Err(e) => Err(e.to_string()),
    }
}

fn global_save(path: &Path, text: &str, revision: Option<&str>) -> Result<String, String> {
    if text.len() as u64 > 32 * 1024 * 1024 {
        return Err("stylesheet is too large".into());
    }
    // atomic_update rechecks the baseline before rename and retries its closure
    // after an external change. Whole-file edits must refuse that retry.
    crate::device_io::atomic_update(path, &GLOBAL_CSS_LOCK, |baseline| {
        let exists = match std::fs::symlink_metadata(path) {
            Ok(_) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        let current = exists.then(|| String::from(FileRev::from_bytes(baseline.as_bytes())));
        if current.as_deref() != revision {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "CSS conflict: reload the disk version before saving",
            ));
        }
        Ok(text.to_owned())
    })
    .map_err(|e| e.to_string())?;
    Ok(FileRev::from_bytes(text.as_bytes()).into())
}

#[tauri::command]
pub(crate) async fn read_css_file(
    global: bool,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<CssFile, String> {
    if global {
        let path = app
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?
            .join("global.css");
        tauri::async_runtime::spawn_blocking(move || global_read(&path))
            .await
            .map_err(|e| e.to_string())?
    } else {
        let slot = slot_for_context(&state)?;
        tauri::async_runtime::spawn_blocking(move || graph_read(&slot.store))
            .await
            .map_err(|e| e.to_string())?
    }
}

#[tauri::command]
pub(crate) async fn save_css_file(
    global: bool,
    text: String,
    revision: Option<String>,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<String, String> {
    if global {
        let directory = app.path().app_config_dir().map_err(|e| e.to_string())?;
        let saved = tauri::async_runtime::spawn_blocking(move || {
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            global_save(&directory.join("global.css"), &text, revision.as_deref())
        })
        .await
        .map_err(|e| e.to_string())??;
        let _ = app.emit(
            "graph-custom-css-changed",
            serde_json::json!({ "global": true }),
        );
        Ok(saved)
    } else {
        let slot = slot_for_context(&state)?;
        tauri::async_runtime::spawn_blocking(move || graph_save(&slot.store, text, revision))
            .await
            .map_err(|e| e.to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn css_edits_refuse_external_replacement_and_concurrent_creation() {
        let dir = tempfile::tempdir().unwrap();
        for area in ["pages", "journals", "logseq"] {
            std::fs::create_dir(dir.path().join(area)).unwrap();
        }
        let store = Store::open(dir.path(), tine_store::OpenOptions::default())
            .unwrap()
            .0;
        let old = graph_read(&store).unwrap();
        std::fs::write(dir.path().join("logseq/custom.css"), "external").unwrap();
        assert!(graph_save(&store, "mine".into(), old.revision).is_err());
        let old = graph_read(&store).unwrap();
        std::fs::write(dir.path().join("logseq/custom.css"), "changed again").unwrap();
        assert!(graph_save(&store, "mine".into(), old.revision).is_err());
        assert_eq!(graph_read(&store).unwrap().text, "changed again");
        let fresh = graph_read(&store).unwrap();
        graph_save(&store, "saved".into(), fresh.revision).unwrap();
        assert_eq!(graph_read(&store).unwrap().text, "saved");
        let path = dir.path().join("global.css");
        global_save(&path, "first", None).unwrap();
        assert!(global_save(&path, "mine", None).is_err());
        let old = global_read(&path).unwrap();
        std::fs::write(&path, "external").unwrap();
        assert!(global_save(&path, "mine", old.revision.as_deref()).is_err());
        assert_eq!(global_read(&path).unwrap().text, "external");
        let fresh = global_read(&path).unwrap();
        global_save(&path, "saved", fresh.revision.as_deref()).unwrap();
        assert_eq!(global_read(&path).unwrap().text, "saved");
    }
}
