//! What every Tauri command can do under Tine-managed storage, enumerated.
//!
//! Managed storage shipped with six working commands. Not because anyone
//! decided the other seventy-eight should fail -- because the six were added one
//! at a time, each when a page needed it, and nothing ever asked the command
//! layer what the rest could do. The gap could only surface as a user report,
//! and it did: "Linked References: the backend request failed", on every page.
//!
//! This module is the missing enumeration. Every `#[tauri::command]` in the
//! sources that can reach a graph slot is listed below with the routing it uses,
//! and the tests re-derive that routing from the sources themselves. Adding a
//! command, or changing how one is routed, is a failing diff here rather than a
//! bug report later.
//!
//! It pins *routing*, which is where the defect lived. A command that reaches a
//! `Graph` can still be refused deeper: the read-only view a managed binding
//! hands out fails every graph-text write at `Graph::admit_managed_text_writer`.
//! `import_native_capture` is the standing example -- it is routed to a `Graph`
//! and then refused inside `tine-core`, because appending a capture to today's
//! journal is a graph-text write the oplog owns.

use std::collections::BTreeMap;

/// How one command reaches a graph, and therefore what a **managed** binding
/// can do with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedRouting {
    /// Has its own managed implementation and routes to the sparse actor.
    ManagedRouted,
    /// Requires the legacy write authority: refused outright under a managed
    /// binding, with `SPARSE_V2_UNSUPPORTED`. This is the list M5 tracks.
    LegacyOnly,
    /// Writes `logseq/config.edn`, which is outside the oplog's document domain.
    ConfigWrite,
    /// Writes the recoverable trash tree, which is outside it too.
    TrashWrite,
    /// Point-addressed filesystem/config/asset service with no retained parsed
    /// page cache and no graph-text write authority.
    Filesystem,
    /// Never touches a graph slot (app settings, platform, plugins, the sparse
    /// runtime's own lifecycle commands).
    NoGraphSlot,
}

use ManagedRouting::{ConfigWrite, Filesystem, LegacyOnly, ManagedRouted, NoGraphSlot, TrashWrite};

/// The sources whose commands can reach a graph slot. The Android media,
/// folder-picker and system-bar commands are deliberately outside this list;
/// `no_graph_routing_hides_in_the_unscanned_sources` proves they hold no graph
/// routing rather than taking it on trust.
const SCANNED_SOURCES: &[(&str, &str)] = &[
    ("backup.rs", include_str!("backup.rs")),
    ("commands.rs", include_str!("commands.rs")),
    ("data_home.rs", include_str!("data_home.rs")),
    ("debug.rs", include_str!("debug.rs")),
    ("graph.rs", include_str!("graph.rs")),
    ("lib.rs", include_str!("lib.rs")),
    (
        "migrate_identifier.rs",
        include_str!("migrate_identifier.rs"),
    ),
    ("platform.rs", include_str!("platform.rs")),
    ("plugins.rs", include_str!("plugins.rs")),
    ("settings.rs", include_str!("settings.rs")),
    ("spellcheck.rs", include_str!("spellcheck.rs")),
    ("sync_runtime.rs", include_str!("sync_runtime.rs")),
    ("watcher.rs", include_str!("watcher.rs")),
];

const UNSCANNED_SOURCES: &[(&str, &str)] = &[
    (
        "android_folder_picker.rs",
        include_str!("android_folder_picker.rs"),
    ),
    ("android_media.rs", include_str!("android_media.rs")),
    (
        "android_system_bars.rs",
        include_str!("android_system_bars.rs"),
    ),
];

/// Every command that can reach a graph slot, and how.
///
/// Sorted by name. Keep it that way; the tests compare sorted sets and the
/// diff is the point.
const MANAGED_COMMAND_SURFACE: &[(&str, ManagedRouting)] = &[
    ("activate_sparse_v2", NoGraphSlot),
    ("app_platform", NoGraphSlot),
    ("apply_spellcheck", NoGraphSlot),
    ("approve_external_assets", NoGraphSlot),
    ("asset_trash_stats", Filesystem),
    ("block_ref_counts", ManagedRouted),
    ("block_referrers", ManagedRouted),
    ("cancel_sparse_v2", NoGraphSlot),
    ("capture_frontend_ready", NoGraphSlot),
    ("capture_graph_binding", NoGraphSlot),
    ("capture_quick_switch", NoGraphSlot),
    ("capture_target", NoGraphSlot),
    ("clipboard_files", NoGraphSlot),
    ("close_graph_window", NoGraphSlot),
    ("copy_guide_into_graph", ManagedRouted),
    ("copy_image_to_clipboard", NoGraphSlot),
    ("create_graph", NoGraphSlot),
    ("debug_info", NoGraphSlot),
    ("debug_log", NoGraphSlot),
    ("default_graph_parent", NoGraphSlot),
    ("delete_page", ManagedRouted),
    ("detect_media_editor", NoGraphSlot),
    ("edit_asset_external", Filesystem),
    ("empty_asset_trash", TrashWrite),
    ("enable_managed_sync", LegacyOnly),
    ("existing_page_names", ManagedRouted),
    ("export_query_subtrees", ManagedRouted),
    ("forget_known_graph", NoGraphSlot),
    ("get_app_bool", NoGraphSlot),
    ("get_app_string", NoGraphSlot),
    ("get_backlink_filter_context", ManagedRouted),
    ("get_backlinks", ManagedRouted),
    ("get_backup_keep", NoGraphSlot),
    ("get_capture_enter_files", NoGraphSlot),
    ("get_link_first_match", NoGraphSlot),
    ("get_page", ManagedRouted),
    ("get_page_by_path", ManagedRouted),
    ("get_smooth_scroll", NoGraphSlot),
    ("get_unlinked_refs", ManagedRouted),
    ("get_watch_mode", NoGraphSlot),
    ("gpu_env", NoGraphSlot),
    ("graph_source_files", Filesystem),
    ("guide_pages", NoGraphSlot),
    ("import_asset", Filesystem),
    ("import_native_capture", Filesystem),
    ("inspect_graph_access", NoGraphSlot),
    ("install_plugin", NoGraphSlot),
    ("join_sparse_v2_shared", NoGraphSlot),
    ("journal_content_days", ManagedRouted),
    ("journal_feed_page", ManagedRouted),
    ("list_backups", LegacyOnly),
    ("list_installed_plugins", NoGraphSlot),
    ("list_journal_conflicts", Filesystem),
    ("list_known_graphs", NoGraphSlot),
    ("list_orphan_assets", ManagedRouted),
    ("list_pages", ManagedRouted),
    ("list_spellcheck_dictionaries", NoGraphSlot),
    ("list_sync_conflicts", Filesystem),
    ("list_templates", ManagedRouted),
    ("load_graph", NoGraphSlot),
    ("load_plugin_registry_cache", NoGraphSlot),
    ("load_session", NoGraphSlot),
    ("load_workspaces", NoGraphSlot),
    ("managed_sync_identity_plan", LegacyOnly),
    ("managed_sync_status", LegacyOnly),
    ("merge_pages", ManagedRouted),
    ("open_asset", Filesystem),
    ("open_external", NoGraphSlot),
    ("open_graph_window", NoGraphSlot),
    ("open_page_file", ManagedRouted),
    ("open_pdf", ManagedRouted),
    ("page_aliases", ManagedRouted),
    ("page_icons", ManagedRouted),
    ("page_print_html", ManagedRouted),
    ("prepare_tine_quit", NoGraphSlot),
    ("prepare_sparse_v2_share", NoGraphSlot),
    ("preview_block", ManagedRouted),
    ("publish_html", ManagedRouted),
    ("query_facets", ManagedRouted),
    ("quick_switch", ManagedRouted),
    ("read_asset", Filesystem),
    ("read_custom_css", Filesystem),
    ("read_highlights", Filesystem),
    ("read_journal_file", Filesystem),
    ("read_local_image", NoGraphSlot),
    ("read_plugin_entry", NoGraphSlot),
    ("read_text_file", NoGraphSlot),
    ("referenced_page_names", ManagedRouted),
    ("rename_file_to_page", ManagedRouted),
    ("rename_page", ManagedRouted),
    ("resolve_block", ManagedRouted),
    ("resolve_blocks", ManagedRouted),
    ("resolve_sync_conflict", ManagedRouted),
    ("restore_backup", LegacyOnly),
    ("run_advanced_query", ManagedRouted),
    ("run_graph_search", ManagedRouted),
    ("run_query", ManagedRouted),
    ("save_asset", Filesystem),
    ("save_page", ManagedRouted),
    ("save_pdf_area_image", Filesystem),
    ("save_session", NoGraphSlot),
    ("save_workspaces", NoGraphSlot),
    ("search", ManagedRouted),
    ("set_app_bool", NoGraphSlot),
    ("set_app_string", NoGraphSlot),
    ("set_backup_keep", LegacyOnly),
    ("set_capture_enter_files", NoGraphSlot),
    ("set_default_journal_template", ConfigWrite),
    ("set_doc_mode_enter_for_new_block", ConfigWrite),
    ("set_favorites", ConfigWrite),
    ("set_guide_announced", ConfigWrite),
    ("set_journal_title_format", ConfigWrite),
    ("set_link_first_match", NoGraphSlot),
    ("set_logical_outdenting", ConfigWrite),
    ("set_plugin_enabled", NoGraphSlot),
    ("set_preferred_format", ConfigWrite),
    ("set_preferred_workflow", ConfigWrite),
    ("set_show_brackets", ConfigWrite),
    ("set_smooth_scroll", NoGraphSlot),
    ("set_start_of_week", ConfigWrite),
    ("set_timetracking_enabled", ConfigWrite),
    ("set_watch_mode", NoGraphSlot),
    ("sparse_v2_clean_shutdown", NoGraphSlot),
    ("sparse_v2_editor_load", NoGraphSlot),
    ("sparse_v2_editor_save", NoGraphSlot),
    ("sparse_v2_query", NoGraphSlot),
    ("sparse_v2_status", NoGraphSlot),
    ("sparse_v2_tick", NoGraphSlot),
    ("startup_graph_path", NoGraphSlot),
    ("store_plugin_registry_cache", NoGraphSlot),
    ("stream_asset_path", Filesystem),
    ("sync_conflict_diff", Filesystem),
    ("take_data_home_fallback_notice", NoGraphSlot),
    ("take_identifier_migration_notice", NoGraphSlot),
    ("tine_open_devtools", NoGraphSlot),
    ("tine_quit", NoGraphSlot),
    ("trash_asset", TrashWrite),
    ("trash_journal_file", ManagedRouted),
    ("trash_sync_conflict", TrashWrite),
    ("uninstall_plugin", NoGraphSlot),
    ("verify_plugin_registry", NoGraphSlot),
    ("warm_done", NoGraphSlot),
    ("write_highlights", ManagedRouted),
    ("write_pdf_view_state", ManagedRouted),
];

/// Every command that a managed binding refuses outright, with the reason it
/// is still refused. Derived from the table above by the tests, and asserted
/// against this list so shrinking it is a deliberate edit.
///
/// Each entry says what the command needs before it can come back.
const REFUSED_UNDER_MANAGED_STORAGE: &[(&str, &str)] = &[
    (
        "enable_managed_sync",
        "legacy-only by construction: it is the migration into managed storage",
    ),
    (
        "list_backups",
        "legacy zip backups have no managed analogue",
    ),
    (
        "managed_sync_identity_plan",
        "legacy-only by construction: plans the migration",
    ),
    (
        "managed_sync_status",
        "legacy-only by construction: reports the migration",
    ),
    (
        "restore_backup",
        "legacy zip backups have no managed analogue",
    ),
    (
        "set_backup_keep",
        "legacy zip backups have no managed analogue",
    ),
];

/// The routing helpers a command body can use, most specific first. A body that
/// dispatches on `sparse_application_handle` has a managed implementation even
/// though its other arm takes the legacy graph, so that marker wins.
const ROUTING_MARKERS: &[(&str, ManagedRouting)] = &[
    ("sparse_application_handle", ManagedRouted),
    ("with_graph(", LegacyOnly),
    ("legacy_graph(", LegacyOnly),
    ("legacy_graph_cloned(", LegacyOnly),
    ("with_config_graph(", ConfigWrite),
    ("with_trash_graph(", TrashWrite),
    ("with_filesystem_graph(", Filesystem),
];

const MARKER_PRECEDENCE: &[ManagedRouting] = &[
    ManagedRouted,
    LegacyOnly,
    ConfigWrite,
    TrashWrite,
    Filesystem,
];

/// Read every `#[tauri::command]` out of one source and classify it by the
/// routing helper its body uses.
fn commands_in(source: &str) -> Vec<(String, ManagedRouting)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() != "#[tauri::command]" {
            index += 1;
            continue;
        }
        let Some(signature) = (index + 1..lines.len()).find(|line| fn_name(lines[*line]).is_some())
        else {
            panic!("`#[tauri::command]` with no function after it at line {index}");
        };
        let name = fn_name(lines[signature]).expect("checked above");
        let mut depth = 0_i64;
        let mut opened = false;
        let mut end = signature;
        for (offset, line) in lines.iter().enumerate().skip(signature) {
            depth += line.matches('{').count() as i64 - line.matches('}').count() as i64;
            opened |= line.contains('{');
            if opened && depth <= 0 {
                end = offset;
                break;
            }
        }
        let body = lines[signature..=end].join("\n");
        let routing = MARKER_PRECEDENCE
            .iter()
            .copied()
            .find(|candidate| {
                ROUTING_MARKERS
                    .iter()
                    .any(|(marker, kind)| kind == candidate && body.contains(marker))
            })
            .unwrap_or(NoGraphSlot);
        found.push((name, routing));
        index = end + 1;
    }
    found
}

/// `fn name(`, `pub(crate) async fn name<R: Runtime>(` and everything between.
fn fn_name(line: &str) -> Option<String> {
    let after = line
        .split_once(" fn ")
        .or_else(|| line.split_once("fn "))?
        .1;
    let name: String = after
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let rest = after[name.len()..].trim_start();
    if name.is_empty() || !(rest.starts_with('(') || rest.starts_with('<')) {
        return None;
    }
    Some(name)
}

fn scanned_surface() -> BTreeMap<String, ManagedRouting> {
    let mut surface = BTreeMap::new();
    for (file, source) in SCANNED_SOURCES {
        for (name, routing) in commands_in(source) {
            if let Some(previous) = surface.insert(name.clone(), routing) {
                panic!("command `{name}` is defined twice ({previous:?} and again in {file})");
            }
        }
    }
    surface
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared() -> BTreeMap<String, ManagedRouting> {
        let mut declared = BTreeMap::new();
        for (name, routing) in MANAGED_COMMAND_SURFACE {
            assert!(
                declared.insert((*name).to_owned(), *routing).is_none(),
                "`{name}` is listed twice in MANAGED_COMMAND_SURFACE"
            );
        }
        declared
    }

    /// The whole point: nothing reaches a graph slot without being listed. A new
    /// command shows up here as a failing diff, and whoever adds it has to say
    /// what it does under managed storage.
    #[test]
    fn every_graph_command_declares_what_managed_storage_can_do_with_it() {
        let scanned = scanned_surface();
        let declared = declared();

        let missing: Vec<_> = scanned
            .keys()
            .filter(|name| !declared.contains_key(*name))
            .collect();
        assert!(
            missing.is_empty(),
            "commands exist that MANAGED_COMMAND_SURFACE does not classify: {missing:?}"
        );
        let stale: Vec<_> = declared
            .keys()
            .filter(|name| !scanned.contains_key(*name))
            .collect();
        assert!(
            stale.is_empty(),
            "MANAGED_COMMAND_SURFACE lists commands that no longer exist: {stale:?}"
        );

        let disagreements: Vec<_> = scanned
            .iter()
            .filter(|(name, routing)| declared.get(*name) != Some(routing))
            .map(|(name, routing)| (name.clone(), declared[name], *routing))
            .collect();
        assert!(
            disagreements.is_empty(),
            "declared routing disagrees with the source (command, declared, actual): {disagreements:?}"
        );
    }

    /// The list M5 tracks. Shrinking it is the work; it must shrink on purpose.
    #[test]
    fn the_refused_surface_is_exactly_the_commands_still_on_the_legacy_authority() {
        let refused: Vec<&str> = scanned_surface()
            .into_iter()
            .filter(|(_, routing)| *routing == LegacyOnly)
            .map(|(name, _)| Box::leak(name.into_boxed_str()) as &str)
            .collect();
        let recorded: Vec<&str> = REFUSED_UNDER_MANAGED_STORAGE
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert!(
            recorded.windows(2).all(|pair| pair[0] < pair[1]),
            "REFUSED_UNDER_MANAGED_STORAGE must stay sorted by name"
        );
        assert_eq!(
            refused, recorded,
            "the set of commands a managed binding refuses changed; \
             update REFUSED_UNDER_MANAGED_STORAGE and say why each one is still refused"
        );
    }

    /// The scanned/unscanned split has to be justified, not assumed. If an
    /// Android command ever reaches a graph slot, this fails and the file joins
    /// `SCANNED_SOURCES`.
    #[test]
    fn no_graph_routing_hides_in_the_unscanned_sources() {
        for (file, source) in UNSCANNED_SOURCES {
            for (marker, _) in ROUTING_MARKERS {
                assert!(
                    !source.contains(marker),
                    "{file} routes through `{marker}` but is not scanned for the managed surface"
                );
            }
        }
    }

    /// Settings and excluded-source trash cleanup are the slice M5-A restored; if
    /// they ever go back onto the legacy authority the diff above would say so,
    /// but name them here too so the regression reads as a sentence.
    #[test]
    fn settings_and_excluded_source_trash_are_not_refused_under_managed_storage() {
        let scanned = scanned_surface();
        for command in [
            "set_favorites",
            "set_preferred_workflow",
            "set_preferred_format",
            "set_journal_title_format",
            "set_start_of_week",
            "set_show_brackets",
            "set_logical_outdenting",
            "set_doc_mode_enter_for_new_block",
            "set_timetracking_enabled",
            "set_default_journal_template",
            "set_guide_announced",
        ] {
            assert_eq!(
                scanned.get(command),
                Some(&ConfigWrite),
                "{command} must persist through the configuration capability"
            );
        }
        for command in ["trash_asset", "empty_asset_trash", "trash_sync_conflict"] {
            assert_eq!(
                scanned.get(command),
                Some(&TrashWrite),
                "{command} must run through the trash capability"
            );
        }
    }
}
