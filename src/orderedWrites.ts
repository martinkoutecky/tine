// R3 (og-flow3 finding 3): the ordered lane for native write commands.
/** R3 (og-flow3): write commands that take the store writer or fsync. They
 *  were synchronous Tauri commands, which the main thread ran one at a time in
 *  issue order (and froze the UI meanwhile); they now run on the native
 *  blocking pool, where two calls are not ordered. This lane issues each only
 *  after the previous one in the lane settled, so their issue order is kept
 *  exactly: a setter toggled twice ends at the second value, a draft retired
 *  after it was stored stays retired. The list must equal `ORDERED_WRITES` in
 *  src-tauri/src/load_wait_guard_tests.rs (checked there). */
export const ORDERED_COMMANDS: ReadonlySet<string> = new Set([
  "save_pages", "save_workspaces", "set_preferred_workflow", "set_timetracking_enabled", "set_show_brackets",
  "set_doc_mode_enter_for_new_block", "set_logical_outdenting", "set_guide_announced",
  "set_default_journal_template", "set_start_of_week", "set_preferred_format", "set_journal_title_format",
  "set_favorites", "set_default_home", "import_asset", "import_native_capture", "empty_asset_trash",
  "trash_journal_file", "save_asset", "save_pdf_area_image", "rollback_pdf_area_image",
  "resolve_sync_conflict", "trash_sync_conflict", "store_draft", "retire_draft", "set_app_bool",
  "set_app_string", "set_capture_enter_files", "set_smooth_scroll", "forget_known_graph", "set_backup_keep",
  "set_watch_mode", "dismiss_defender_hint", "approve_external_assets", "create_graph", "install_plugin",
  "uninstall_plugin", "set_plugin_enabled", "store_plugin_registry_cache", "diagnostic_session_active",
  "clear_diagnostics",
]);

/** A lane issues an ORDERED_COMMANDS call only after the previous one in the
 *  lane settled (either way); the tail is taken synchronously, so the lane
 *  order is the call order. Any other command is issued at once. A failure
 *  rejects only its own call and never jams the lane. Cost: O(1) per call. */
export function orderedLane(): <T>(cmd: string, issue: () => Promise<T>) => Promise<T> {
  let tail: Promise<unknown> = Promise.resolve();
  return <T>(cmd: string, issue: () => Promise<T>): Promise<T> => {
    if (!ORDERED_COMMANDS.has(cmd)) return issue();
    const run = tail.then(issue);
    tail = run.then(() => undefined, () => undefined);
    return run;
  };
}
