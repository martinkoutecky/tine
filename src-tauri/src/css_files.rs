//! CSS editing and desktop handoff: fixed paths and existing guarded writers.
use crate::state::{slot_for_context, GraphContext};
use std::{io, path::Path, sync::Mutex};
use tauri::{Emitter, Manager};
use tine_store::{Area, Content, FileRev, Store, StoreError, TxOutcome};

const MAX_CSS: u64 = tine_store::PARSE_INPUT_MAX_BYTES;
static GLOBAL_CSS_LOCK: Mutex<()> = Mutex::new(());

/// Watch the directory so temp-file + rename saves, deletion and recreation
/// keep working. Access events (including our reads) never trigger a reload.
fn watch_global_css(
    directory: &Path,
    changed: impl Fn() + Send + 'static,
) -> Result<notify::RecommendedWatcher, String> {
    use notify::Watcher;
    let path = directory.join("global.css");
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        match event {
            Ok(event)
                if matches!(
                    event.kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                ) && event.paths.contains(&path) =>
            {
                changed()
            }
            // A watcher error can mean missed events. Re-read, preserving the
            // frontend's last good CSS if that read also fails.
            Err(_) => changed(),
            _ => {}
        }
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(directory, notify::RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;
    Ok(watcher)
}

pub(crate) fn install_global_css_watcher(app: &tauri::AppHandle) -> Result<(), String> {
    let directory = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let handle = app.clone();
    let watcher = watch_global_css(&directory, move || {
        let _ = handle.emit(
            "graph-custom-css-changed",
            serde_json::json!({ "global": true }),
        );
    })?;
    // Retain the watch for the app lifetime, independently of graph bindings.
    app.manage(Mutex::new(watcher));
    Ok(())
}

fn ensure_global_css(path: &Path) -> Result<(), String> {
    // Like graph CSS, existing files need only be regular, not valid UTF-8 or
    // small enough for the in-app editor. The external editor can repair them.
    match crate::device_io::read_bounded(path, 0) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Same guarded device writer as the in-app editor: concurrent
            // creation is refused, never replaced with an empty stylesheet.
            global_save(path, "", None).map(|_| ())
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(desktop)]
pub(crate) async fn open_css_with_os(
    app: tauri::AppHandle,
    path: std::path::PathBuf,
) -> Result<(), String> {
    // A foreground editor may keep xdg-open alive. One reaper thread waits for
    // the actual verdict, including a delayed failure, without holding a Tauri
    // blocking worker or a graph/store lock. The async IPC caller stays pending.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("css-opener".into())
        .spawn(move || {
            let result = css_opener_result(&app, &path);
            let _ = sender.send(result);
        })
        .map_err(|e| e.to_string())?;
    receiver.await.map_err(|e| e.to_string())?
}

#[cfg(desktop)]
fn css_opener_result(app: &tauri::AppHandle, path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_opener::OpenerExt;
        // ShellExecute reports a missing association rather than opening an
        // unrelated Explorer window and pretending the handoff succeeded.
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let _ = app;
        #[cfg(target_os = "linux")]
        let mut command = crate::platform::opener_command("xdg-open");
        #[cfg(target_os = "macos")]
        let mut command = crate::platform::opener_command("open");
        checked_opener(command.arg(path))
    }
}

/// Called on the dedicated reaper thread. Never turn a launched process into
/// success before it reports whether the OS accepted the handoff.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn checked_opener(command: &mut std::process::Command) -> Result<(), String> {
    let status = command.status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("default app launcher failed: {status}"))
    }
}

#[tauri::command]
pub(crate) async fn edit_global_css(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err("global.css opens in a desktop editor".into())
    }
    #[cfg(desktop)]
    {
        let directory = app.path().app_config_dir().map_err(|e| e.to_string())?;
        let path = tauri::async_runtime::spawn_blocking(move || {
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            let path = directory.join("global.css");
            ensure_global_css(&path)?;
            Ok::<_, String>(path)
        })
        .await
        .map_err(|e| e.to_string())??;
        open_css_with_os(app, path).await
    }
}

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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn css_opener_reports_a_launcher_refusal_without_starting_an_os_opener() {
        let error =
            checked_opener(std::process::Command::new("sh").args(["-c", "exit 7"])).unwrap_err();
        assert!(error.contains("default app launcher failed"));
        assert!(error.contains('7'));
        checked_opener(std::process::Command::new("sh").args(["-c", "exit 0"])).unwrap();
    }
    #[test]
    fn global_css_handoff_creates_without_overwriting_and_refuses_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("global.css");
        ensure_global_css(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        std::fs::write(&path, "external editor text").unwrap();
        ensure_global_css(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "external editor text"
        );
        std::fs::write(&path, [0xff]).unwrap();
        ensure_global_css(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [0xff]);
        let directory = dir.path().join("not-a-file");
        std::fs::create_dir(&directory).unwrap();
        assert!(ensure_global_css(&directory).is_err());
    }

    #[test]
    fn external_global_css_saves_delete_and_recreate_publish_changes() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("global.css");
        let (sender, receiver) = std::sync::mpsc::channel();
        let _watcher = watch_global_css(dir.path(), move || {
            let _ = sender.send(());
        })
        .unwrap();
        let changed = || receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let drain = || {
            while receiver.try_recv().is_ok() {}
        };
        std::fs::write(&path, "first").unwrap();
        changed();
        assert_eq!(global_read(&path).unwrap().text, "first");
        drain();
        std::fs::write(&path, "in-place").unwrap();
        changed();
        assert_eq!(global_read(&path).unwrap().text, "in-place");
        drain();
        crate::device_io::atomic_write(&path, b"renamed").unwrap();
        changed();
        assert_eq!(global_read(&path).unwrap().text, "renamed");
        drain();
        std::fs::remove_file(&path).unwrap();
        changed();
        assert!(global_read(&path).unwrap().revision.is_none());
        drain();
        std::fs::write(&path, "recreated").unwrap();
        changed();
        assert_eq!(global_read(&path).unwrap().text, "recreated");
    }
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
