//! The registered Tauri command surface, as data.
//!
//! **Question answered.** [`is_known_command`] — is this string the name of a
//! command `lib.rs` registers? The diagnostic recorder uses it so a frontend
//! IPC timing event can carry a command name without ever carrying free text
//! (I-5). The list is kept equal to the `generate_handler!` list by a guard
//! test; adding a command means adding its name here.

/// Every registered command's name (last path segment), sorted.
pub(crate) const KNOWN_COMMANDS: [&str; 183] = [
    "add_defender_exclusion",
    "app_architecture",
    "app_platform",
    "apply_journal_filename_migrations",
    "apply_spellcheck",
    "approve_external_assets",
    "asset_trash_stats",
    "block_ref_counts",
    "block_referrers",
    "cancel_graph_verification",
    "cancel_recording",
    "capture_frontend_ready",
    "capture_graph_binding",
    "capture_photo",
    "capture_quick_switch",
    "capture_target",
    "clear_diagnostics",
    "clipboard_files",
    "close_graph_window",
    "close_search_workspace",
    "conflict_inventory",
    "copy_guide_into_graph",
    "copy_image_to_clipboard",
    "create_graph",
    "create_graph_verification",
    "debug_info",
    "debug_log",
    "default_graph_parent",
    "defender_hint",
    "delete_page",
    "detect_media_editor",
    "diagnostic_frontend_event",
    "diagnostic_ipc_event",
    "diagnostic_report",
    "diagnostic_session_active",
    "diagnostic_timing_event",
    "dismiss_defender_hint",
    "duplicate_journal_diff",
    "edit_asset_external",
    "edit_custom_css",
    "empty_asset_trash",
    "export_query_subtrees",
    "forget_known_graph",
    "get_app_bool",
    "get_app_string",
    "get_backlink_filter_context",
    "get_backlinks",
    "get_backup_keep",
    "get_capture_enter_files",
    "get_link_first_match",
    "get_page",
    "get_page_by_path",
    "get_smooth_scroll",
    "get_unlinked_refs",
    "get_watch_mode",
    "gpu_env",
    "graph_link_identity",
    "graph_source_files",
    "guide_pages",
    "handoff_tine_link",
    "import_asset",
    "import_native_capture",
    "inspect_graph_access",
    "install_plugin",
    "journal_content_days",
    "journal_feed_page",
    "list_backups",
    "list_installed_plugins",
    "list_journal_conflicts",
    "list_journal_filename_migrations",
    "list_known_graphs",
    "list_orphan_assets",
    "list_spellcheck_dictionaries",
    "list_sync_conflicts",
    "list_templates",
    "live_conflict_diff",
    "load_drafts",
    "load_graph",
    "load_plugin_registry_cache",
    "load_session",
    "load_workspaces",
    "local_clock",
    "merge_pages",
    "open_asset",
    "open_external",
    "open_graph_window",
    "open_page_file",
    "open_pdf",
    "page_icons",
    "page_inventory",
    "page_print_html",
    "pick_graph_folder",
    "prepare_graph_folder",
    "preview_block",
    "publish_html",
    "publish_live",
    "publish_query",
    "publish_query_plan",
    "query_explain_empty",
    "query_facets",
    "query_og_expressible",
    "query_parse",
    "query_print",
    "query_registry",
    "query_run",
    "quick_switch",
    "read_asset",
    "read_custom_css",
    "read_highlights",
    "read_journal_file",
    "read_local_image",
    "read_plugin_entry",
    "read_text_file",
    "rename_file_to_page",
    "rename_page",
    "rescan_graph_now",
    "resolve_block",
    "resolve_blocks",
    "resolve_duplicate_journal_day",
    "resolve_live_conflict",
    "resolve_page",
    "resolve_sync_conflict",
    "resolve_vcs_marker_conflict",
    "restore_backup",
    "retire_draft",
    "reveal_known_graph",
    "rollback_pdf_area_image",
    "run_graph_search",
    "save_asset",
    "save_diagnostic_report",
    "save_graph_verification_report",
    "save_pages",
    "save_pdf_area_image",
    "save_session",
    "save_workspaces",
    "scan_known_graphs_for_link",
    "search",
    "set_app_bool",
    "set_app_string",
    "set_backup_keep",
    "set_capture_enter_files",
    "set_default_home",
    "set_default_journal_template",
    "set_doc_mode_enter_for_new_block",
    "set_favorites",
    "set_guide_announced",
    "set_journal_title_format",
    "set_logical_outdenting",
    "set_plugin_enabled",
    "set_preferred_format",
    "set_preferred_workflow",
    "set_show_brackets",
    "set_smooth_scroll",
    "set_start_of_week",
    "set_system_bar_appearance",
    "set_timetracking_enabled",
    "set_watch_mode",
    "sheet_export_inputs",
    "start_recording",
    "startup_graph_path",
    "stop_recording",
    "store_draft",
    "store_plugin_registry_cache",
    "stream_asset_path",
    "suggest_graph_name",
    "sync_conflict_diff",
    "take_data_home_fallback_notice",
    "take_identifier_migration_notice",
    "take_tine_links",
    "tine_open_devtools",
    "tine_quit",
    "trash_asset",
    "trash_journal_file",
    "trash_sync_conflict",
    "tray_apply",
    "uninstall_plugin",
    "vcs_marker_conflict_diff",
    "verify_plugin_registry",
    "warm_done",
    "watcher_latency_recent",
    "workspace_window_destroy",
    "workspace_window_prepare",
    "write_highlights",
];

/// Whether  is exactly a registered command name. O(log n); pure.
pub(crate) fn is_known_command(name: &str) -> bool {
    KNOWN_COMMANDS.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every name registered in `lib.rs`, module paths and `cfg` attributes
    /// stripped, on every target (master's parser: a `#[cfg(...)]` inside the
    /// handler list contains `]`, so the list end is found by bracket depth).
    fn registered() -> BTreeSet<String> {
        let lib = include_str!("lib.rs");
        let start =
            lib.find("generate_handler![").expect("handler list") + "generate_handler![".len();
        let mut depth = 1usize;
        let mut end = start;
        for (offset, byte) in lib[start..].bytes().enumerate() {
            match byte {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        lib[start..end]
            .lines()
            .map(|line| line.trim().trim_end_matches(','))
            .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with("#["))
            .map(|line| line.rsplit("::").next().unwrap_or(line).trim().to_string())
            .collect()
    }

    #[test]
    fn the_known_command_list_is_exactly_the_registered_handler_list() {
        let known: BTreeSet<String> = KNOWN_COMMANDS.iter().map(|name| name.to_string()).collect();
        assert_eq!(known.len(), KNOWN_COMMANDS.len(), "duplicate command name");
        assert!(
            KNOWN_COMMANDS.windows(2).all(|pair| pair[0] < pair[1]),
            "KNOWN_COMMANDS must stay sorted so is_known_command can binary-search"
        );
        assert_eq!(
            known,
            registered(),
            "KNOWN_COMMANDS must equal lib.rs generate_handler! across every target's cfg arm (sorted); a diagnostic IPC event may name only a registered command (I-5)"
        );
    }

    #[test]
    fn free_text_is_not_a_command() {
        assert!(is_known_command("save_pages"));
        assert!(!is_known_command("My secret page"));
        assert!(!is_known_command(""));
    }
}
