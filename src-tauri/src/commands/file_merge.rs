use super::rename::RewriteError;
use super::*;

/// Reconcile two graph-relative page files through the guarded merge path.
/// `save_first` is the user's explicit unchanged-save recovery step; failure
/// stops here, before the merge, while a successful save remains on disk.
#[tauri::command]
pub(crate) async fn merge_pages(
    src: String,
    dst: String,
    save_first: Option<String>,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<(), RewriteError> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::git_settings::around(
            &slot.root_key,
            || format!("Merge \"{src}\" into \"{dst}\""),
            || {
                if let Some(path) = save_first.as_deref() {
                    tine_graph_features::pages::save_page_unchanged(
                        &slot.store,
                        slot.host_slot()?.running()?,
                        path,
                    )
                    .map_err(RewriteError::of)?;
                }
                tine_graph_features::pages::merge_pages(
                    &slot.store,
                    slot.host_slot()?.running()?,
                    &src,
                    &dst,
                )
                .map_err(RewriteError::of)
            },
        )
        .inspect(|()| crate::seen::page_gone(&app, &slot, None, &[src.as_str()]))
    })
    .await
    .map_err(|error| RewriteError::from(error.to_string()))?
}
