//! A page's name changed without a rename: the config step that follows it,
//! kept out of `commands.rs` (an oversized file the size ratchet holds).

use crate::state::{slot_for_context, GraphContext};

/// A page's name changed without a rename (its `title::` was edited, in this
/// window or outside): the backend moves home and favorites with it through
/// the rename's config step and answers the lists the window adopts
/// (`pages::follow_page_identity`, CM step 2 FAV-RETITLE). `path` is the
/// page's file, `old` the name the window knew it by.
#[tauri::command]
pub(crate) async fn follow_page_identity(
    path: String,
    old: String,
    state: GraphContext<'_>,
) -> Result<tine_graph_features::pages::IdentityFollow, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::follow_page_identity(&slot.store, &path, &old)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    /// The follow reads and may rewrite config.edn: graph-wide store work,
    /// never on the command/UI thread (the rename family's rule,
    /// `commands.rs` `graph_wide_command_boundary_tests`).
    #[test]
    fn follow_page_identity_crosses_the_blocking_pool() {
        let source = include_str!("identity_follow.rs");
        let start = source
            .find("pub(crate) async fn follow_page_identity(")
            .expect("command stays async");
        let body = &source[start..source.find("#[cfg(test)]").unwrap()];
        assert!(
            body.contains("tauri::async_runtime::spawn_blocking"),
            "follow_page_identity must not run graph-wide work on the command/UI thread"
        );
    }
}
