//! The seen baseline's IPC (ADR 0079), on all five shipped targets.
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::owner::seen as imp;

#[derive(Debug, Serialize)]
pub(crate) struct SeenBlock {
    pub block_path: Vec<usize>,
    pub persisted_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SeenRemoved {
    pub text: String,
    pub baseline_block_path: Vec<usize>,
    pub parent_path: Option<Vec<usize>>,
    pub previous_sibling_path: Option<Vec<usize>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SeenChanges {
    pub available: bool,
    pub reason: Option<String>,
    pub head_commit: Option<String>,
    pub baseline_commit: Option<String>,
    pub current_text_sha256: String,
    pub identity_known: bool,
    pub unattributed: bool,
    pub blocks: Vec<SeenBlock>,
    pub removed: Vec<SeenRemoved>,
}

impl SeenChanges {
    pub(crate) fn available(text: &str) -> Self {
        Self {
            available: true,
            reason: None,
            head_commit: None,
            baseline_commit: None,
            current_text_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
            identity_known: false,
            unattributed: false,
            blocks: Vec::new(),
            removed: Vec::new(),
        }
    }
    pub(crate) fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            ..Self::available("")
        }
    }
}

pub(crate) fn bound(app: &tauri::AppHandle, slot: &crate::state::GraphSlot) {
    if let Some(data) = crate::graph::checkpoint_app_data(app) {
        // Binding already runs off the UI thread. Failure never refuses a graph.
        if let Err(error) = imp::initialize(&data, &slot.root_key) {
            crate::debug::diag_private("seen-initialize", error);
        }
    }
}

#[tauri::command]
pub(crate) async fn get_page_changes(
    page_key: String,
    path: String,
    current_text: String,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<SeenChanges, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || {
        let Some(data) = crate::graph::checkpoint_app_data(&app) else {
            return Ok(SeenChanges::unavailable(
                "No app-data directory is available.",
            ));
        };
        // The same "Your name" the git writers read (ADR 0078 identity).
        imp::changes(
            &data,
            &slot.root_key,
            &page_key,
            &path,
            &current_text,
            &crate::settings::your_name(),
        )
    })
    .await
}

#[tauri::command]
pub(crate) async fn mark_page_seen(
    page_key: String,
    path: String,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<(), String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || {
        let data =
            crate::graph::checkpoint_app_data(&app).ok_or("No app-data directory is available.")?;
        imp::mark(&data, &slot.root_key, Some((&page_key, &path)))
    })
    .await
}

#[tauri::command]
pub(crate) async fn mark_graph_seen(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<(), String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || {
        let data =
            crate::graph::checkpoint_app_data(&app).ok_or("No app-data directory is available.")?;
        imp::mark(&data, &slot.root_key, None)
    })
    .await
}

/// Tine's own rename, merge or delete of a page drops that page's seen
/// override (D-20261017-SB1 call 6). Called on the blocking thread that ran
/// the operation, after it. The record is device-local and disposable: a
/// failure is logged and never fails or blocks the page operation.
pub(crate) fn page_gone(
    app: &tauri::AppHandle,
    slot: &crate::state::GraphSlot,
    name: Option<&str>,
    paths: &[&str],
) {
    let Some(data) = crate::graph::checkpoint_app_data(app) else {
        return;
    };
    let key = name.map(tine_core::refs::page_key);
    let paths: Vec<Option<&str>> = if paths.is_empty() {
        vec![None]
    } else {
        paths.iter().map(|p| Some(*p)).collect()
    };
    for path in paths {
        if let Err(error) = imp::forget_page(&data, &slot.root_key, key.as_deref(), path) {
            crate::debug::diag_private("seen-forget-page", error);
        }
    }
}

/// [`page_gone`] for a rename: the old name, and every file the rename moved.
pub(crate) fn page_renamed(
    app: &tauri::AppHandle,
    slot: &crate::state::GraphSlot,
    old: &str,
    report: &tine_graph_features::pages::RenameReport,
) {
    let moved: Vec<&str> = report
        .touched
        .iter()
        .filter(|page| page.moved)
        .map(|page| page.path.as_str())
        .collect();
    page_gone(app, slot, Some(old), &moved);
}

#[tauri::command]
pub(crate) async fn forget_seen_state(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<(), String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || {
        let data =
            crate::graph::checkpoint_app_data(&app).ok_or("No app-data directory is available.")?;
        imp::forget(&data, &slot.root_key)
    })
    .await
}

#[cfg(test)]
mod tests {
    /// D-20261017-SB1 call 6: Tine's own rename, merge and delete drop the
    /// page's seen record, after the operation and without failing it.
    #[test]
    fn seen_record_is_dropped_by_tine_rename_merge_and_delete() {
        let commands = include_str!("commands.rs");
        let pages = include_str!("page_commands.rs");
        let body = |source: &'static str, name: &str| {
            let start = source
                .find(&format!("pub(crate) async fn {name}("))
                .unwrap();
            let tail = &source[start..];
            &tail[..tail.find("\n}\n").unwrap()]
        };
        assert!(body(commands, "rename_page").contains("crate::seen::page_renamed(&app, &slot"));
        let renamed = &include_str!("seen.rs")[include_str!("seen.rs")
            .find("pub(crate) fn page_renamed")
            .unwrap()..];
        assert!(
            renamed[..renamed.find("\n}\n").unwrap()].contains("page_gone(app, slot, Some(old)")
        );
        for name in ["merge_pages", "rename_file_to_page"] {
            assert!(
                body(commands, name).contains("crate::seen::page_gone(&app, &slot"),
                "{name}"
            );
        }
        assert!(body(pages, "page_delete").contains("crate::seen::page_gone(&app, &slot"));
        let gone = &include_str!("seen.rs")[include_str!("seen.rs")
            .find("pub(crate) fn page_gone")
            .unwrap()..];
        let gone = &gone[..gone.find("\n}\n").unwrap()];
        assert!(
            gone.contains("diag_private") && !gone.contains('?'),
            "never fails the page operation"
        );
    }
}
