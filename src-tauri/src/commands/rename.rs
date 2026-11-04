//! The page rename commands: the rename dialog's question (`rename_offer`)
//! and the rename itself (`rename_page`), both answered by
//! `tine_graph_features::pages` (one planner; the window never re-derives a
//! rename rule). Split out of `commands.rs` along this seam (CM step 2
//! batch 12, PARITY-CAMPAIGN §3).
use super::*;

/// What renaming `old` to `new` asks of the user (the rename dialog's
/// question): a plain rename, a merge into the page holding `new`, the M-Q5
/// three-way choice when `new` is only an alias, or a refusal with its reason.
/// The rename's own planner answers (`pages::rename_offer`); the window never
/// re-derives it. Read-only.
#[tauri::command]
pub(crate) async fn rename_offer(
    old: String,
    new: String,
    expected_path: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_graph_features::pages::RenameOffer, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::pages::rename_offer(&slot.store, &old, &new, expected_path.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Rename `old` to `new`. `merge_into`: the user confirmed a merge into that
/// page file. `own_page`: the user chose "make it its own page" for a name
/// that is only an alias (M-Q5); it wins over `merge_into`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn rename_page(
    old: String,
    new: String,
    expected_path: Option<String>,
    merge_into: Option<String>,
    own_page: Option<bool>,
    unsaved_paths: Option<Vec<String>>,
    app: tauri::AppHandle,
    state: GraphContext<'_>,
) -> Result<tine_graph_features::pages::RenameReport, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        // A rename is the page host's (STEP3 §7): a revoked slot is the
        // stale-binding refusal, an off one refuses with no write. One
        // statement-temporary guard (A-K3, `host_slot_guard_tests.rs`).
        // On a git graph that autocommits, bracketed by commits (ADR 0078).
        crate::git_settings::around(
            &slot.root_key,
            || match (&merge_into, own_page) {
                (Some(_), None | Some(false)) => {
                    format!("Merge page \"{old}\" into \"{new}\"")
                }
                _ => format!("Rename page \"{old}\" to \"{new}\""),
            },
            || {
                use tine_graph_features::pages::RenameChoice;
                let choice = match (own_page, merge_into.as_deref()) {
                    (Some(true), _) => RenameChoice::OwnPage,
                    (_, Some(into)) => RenameChoice::MergeInto(into),
                    (_, None) => RenameChoice::Rename,
                };
                tine_graph_features::pages::rename_page_as(
                    &slot.store,
                    slot.host_slot()?.running()?,
                    &old,
                    &new,
                    expected_path.as_deref(),
                    choice,
                    unsaved_paths.as_deref().unwrap_or_default(),
                )
                .map_err(|e| e.to_string())
            },
        )
        .inspect(|report| crate::seen::page_renamed(&app, &slot, &old, report))
    })
    .await
    .map_err(|error| error.to_string())?
}
