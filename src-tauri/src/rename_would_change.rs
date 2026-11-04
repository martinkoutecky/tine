//! The stuck-page check of a rename (GH #535) asks the rename itself, kept out
//! of `commands.rs` (an oversized file the size ratchet holds).

use crate::state::{slot_for_context, GraphContext};

/// Per text an unsaved page holds, whether renaming `old` to `new` would
/// rewrite it:
/// the rename's own answer, asked before a rename that must leave a stuck
/// page alone (GH #535). Read-only; O(texts + `old`'s namespace).
#[tauri::command]
pub(crate) async fn rename_would_change(
    old: String,
    new: String,
    texts: Vec<tine_graph_features::pages::RenameProbeText>,
    state: GraphContext<'_>,
) -> Result<Vec<bool>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::rename_would_change(&slot.store, &old, &new, &texts)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    /// The answer reads the planning view: graph-wide store work, never on the
    /// command/UI thread (the rename family's rule, `commands.rs`
    /// `graph_wide_command_boundary_tests`).
    #[test]
    fn rename_would_change_crosses_the_blocking_pool() {
        let source = include_str!("rename_would_change.rs");
        let start = source
            .find("pub(crate) async fn rename_would_change(")
            .expect("command stays async");
        let body = &source[start..source.find("#[cfg(test)]").unwrap()];
        assert!(
            body.contains("tauri::async_runtime::spawn_blocking"),
            "rename_would_change must not run graph-wide work on the command/UI thread"
        );
    }
}
