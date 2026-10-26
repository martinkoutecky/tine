// R3 (og-flow3 finding 3): the ordered lane for native write commands.
/** R3 (og-flow3): write commands that take the store writer or fsync. They
 *  were synchronous Tauri commands, which the main thread ran one at a time in
 *  issue order (and froze the UI meanwhile); they now run on the native
 *  blocking pool, where two calls are not ordered. This lane issues each only
 *  after the previous one in the lane settled, so their issue order is kept
 *  exactly: a setter toggled twice ends at the second value. The page host
 *  admits a window's requests only in id order (`page_open` … `page_close`
 *  carry ids); its queries (`page_wait`, which blocks, `page_owed`,
 *  `page_save_now`) and `page_delete` carry none and stay out of the lane.
 *  The list must equal `ORDERED_WRITES` in
 *  src-tauri/src/load_wait_guard_tests.rs (checked there). */
export const ORDERED_COMMANDS: ReadonlySet<string> = new Set([
  "page_open", "page_submit", "page_move", "page_discard", "page_close", "save_workspaces",
  "set_preferred_workflow", "set_timetracking_enabled", "set_show_brackets",
  "set_doc_mode_enter_for_new_block", "set_logical_outdenting", "set_guide_announced",
  "set_default_journal_template", "set_start_of_week", "set_preferred_format", "set_journal_title_format",
  "set_favorites", "set_default_home", "import_asset", "import_native_capture", "empty_asset_trash",
  "trash_journal_file", "save_asset", "save_pdf_area_image", "rollback_pdf_area_image",
  "resolve_sync_conflict", "trash_sync_conflict", "set_app_bool",
  "set_app_string", "set_capture_enter_files", "set_smooth_scroll", "forget_known_graph", "set_backup_keep",
  "set_watch_mode", "dismiss_defender_hint", "approve_external_assets", "create_graph", "install_plugin",
  "uninstall_plugin", "set_plugin_enabled", "store_plugin_registry_cache", "diagnostic_session_active",
  "clear_diagnostics", "set_ui_locale",
]);

/** An ordered lane: `lane(cmd, issue)` for one call, `lane.joined` to join a
 *  batch. */
export interface OrderedLane {
  <T>(cmd: string, issue: () => Promise<T>): Promise<T>;
  /** PERF-FIXES D3: join `item` to the lane's last batch of `key` when nothing
   *  else entered the lane after it and it has not started, else start a new
   *  batch there. `issue` gets the batch's items in call order and answers
   *  each, so joining never moves a call past another one. A failure rejects
   *  every call of its batch. */
  joined<I, R>(key: string, item: I, issue: (items: I[]) => Promise<R[]>): Promise<R>;
}

/** A lane issues an ORDERED_COMMANDS call only after the previous one in the
 *  lane settled (either way); the tail is taken synchronously, so the lane
 *  order is the call order. Any other command is issued at once. A failure
 *  rejects only its own call (or batch) and never jams the lane. Cost: O(1)
 *  per call. */
export function orderedLane(): OrderedLane {
  let tail: Promise<unknown> = Promise.resolve();
  let open: { key: string; items: unknown[]; run: Promise<unknown[]> } | null = null;
  const lane = <T>(cmd: string, issue: () => Promise<T>): Promise<T> => {
    if (!ORDERED_COMMANDS.has(cmd)) return issue();
    open = null;
    const run = tail.then(issue);
    tail = run.then(() => undefined, () => undefined);
    return run;
  };
  const joined = <I, R>(key: string, item: I, issue: (items: I[]) => Promise<R[]>): Promise<R> => {
    if (open?.key === key) {
      const index = open.items.push(item) - 1;
      return (open.run as Promise<R[]>).then((answers) => answers[index]);
    }
    const items: I[] = [item];
    const batch: { key: string; items: unknown[]; run: Promise<unknown[]> } = { key, items, run: Promise.resolve([]) };
    batch.run = tail.then(() => {
      if (open === batch) open = null;
      return issue(items);
    });
    open = batch;
    tail = batch.run.then(() => undefined, () => undefined);
    return (batch.run as Promise<R[]>).then((answers) => answers[0]);
  };
  return Object.assign(lane, { joined });
}
