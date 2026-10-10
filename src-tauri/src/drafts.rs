//! The v1 crash-draft store's legacy file (og ADR 0061, family 9; GH #540).
//!
//! Crash-surviving drafts are the page host's (`drafts-v2`, STEP3 §9). The v1
//! store, one file per graph at `<app data>/drafts/<graph-id>.v1.json`, is no
//! longer read or written: an existing file is left untouched on disk as a
//! backup, and the window shows a one-time notice naming it (D-1).
//!
//! **Question answered.** [`legacy_drafts_file`]: where is this graph's v1
//! draft file, if it exists? No read of its content, no write.
//!
//! **Cost.** One `exists` probe of one path.

use std::path::Path;
use tauri::Manager;

/// The v1 store file of the graph whose root key is `root`.
fn drafts_file_name(root: &Path) -> String {
    let id = crate::settings::session_id(root);
    let stem = id.strip_suffix(".json").unwrap_or(&id);
    format!("{stem}.v1.json")
}

/// The absolute path of the window's graph's v1 draft file when it exists
/// (D-1: the one-time notice names it); `None` otherwise. Read-only.
#[tauri::command]
pub(crate) fn legacy_drafts_file(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<Option<String>, String> {
    let slot = crate::state::slot_for_context(&state)?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let path = dir.join("drafts").join(drafts_file_name(&slot.root_key));
    Ok(path
        .try_exists()
        .map_err(|e| e.to_string())?
        .then(|| path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn each_graph_root_names_its_own_v1_file() {
        // The v1 store keyed a graph by `graph_meta`'s root, `root_key.display()`.
        let a = PathBuf::from("/home/u/graphs/notes");
        let b = PathBuf::from("/home/u/other/notes");
        let named = |root: &Path| drafts_file_name(Path::new(&root.display().to_string()));
        assert_eq!(named(&a), drafts_file_name(&a));
        assert_ne!(named(&a), named(&b));
        assert!(named(&a).ends_with(".v1.json"), "{}", named(&a));
    }
}
