//! I-13: Tauri commands that can wait for a graph load run off the UI thread.

fn commands(source: &str) -> Vec<(String, bool, String)> {
    source
        .split("#[tauri::command]")
        .skip(1)
        .filter_map(|tail| {
            let header = tail.find("fn ")?;
            let asynchronous = tail[..header].contains("async ");
            let name = tail[header + 3..].split_once('(')?.0.to_owned();
            let open = tail[header..].find('{')? + header;
            let mut depth = 0;
            for (offset, byte) in tail.as_bytes()[open..].iter().enumerate() {
                if *byte == b'{' {
                    depth += 1;
                }
                if *byte == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        return Some((
                            name,
                            asynchronous,
                            tail[open..open + offset + 1].to_owned(),
                        ));
                    }
                }
            }
            panic!("unclosed Tauri command {name}")
        })
        .collect()
}

fn reaches_load_wait(body: &str) -> bool {
    // Include the direct store calls and feature functions that delegate to
    // Store::whole_graph / scan_refresh / the cold-cache publication wait.
    [
        ".whole_graph(",
        ".scan_refresh(",
        // Opens a Store, joins the startup-open worker, and tears the
        // displaced graph's Store down (~200 ms for a Ready graph, measured on
        // a copy of the anonymized graph; master abf7af831884).
        "load_graph_for_label(",
        "capture_quick_switch_for(",
        "tine_graph_features::pages::delete_page_expected(",
        "tine_graph_features::pages::merge_pages(",
        "tine_graph_features::pages::rename_file_to_page(",
        "tine_graph_features::pages::source_path_for_os_handoff(",
        "tine_graph_features::guide::copy_guide_into_graph(",
        "tine_graph_features::print::page_print_html(",
        "tine_graph_features::print::page_print_html_with_sheets(",
        "tine_graph_features::publish::sheet_export_inputs(",
        "tine_graph_features::pdf::open_pdf(",
        "tine_graph_features::pdf::write_highlights(",
    ]
    .iter()
    .any(|marker| body.contains(marker))
}

#[test]
fn load_waiting_tauri_commands_are_async_and_leave_the_ui_thread() {
    let source = include_str!("commands.rs");
    let listed = commands(source);
    let other_sources = [
        include_str!("graph.rs"),
        include_str!("commands/concord.rs"),
        include_str!("backup.rs"),
        include_str!("backup/restore.rs"),
        include_str!("settings.rs"),
        include_str!("watcher.rs"),
        include_str!("plugins.rs"),
        include_str!("lib.rs"),
    ];
    let all = listed
        .iter()
        .cloned()
        .chain(other_sources.iter().flat_map(|source| commands(source)));
    for (name, asynchronous, body) in all {
        if reaches_load_wait(&body) {
            assert!(asynchronous, "I-13 / OG-RULES Rule 4: {name} reaches the initial graph-load wait; a synchronous Tauri command blocks the UI thread. Make it async and use the blocking pool; exemplar resolve_blocks");
            assert!(body.contains("spawn_blocking(") || body.contains("off_ui_graph_read("),
                "I-13 / OG-RULES Rule 4: {name} must run load-waiting work on the blocking pool; exemplar resolve_blocks");
        }
    }
    // A new synchronous command needs an explicit load-path audit here. This
    // closed list prevents an indirect helper from silently adding a wait.
    const AUDITED_SYNC: &[&str] = &[
        "load_workspaces",
        "save_workspaces",
        "save_pages",
        "guide_pages",
        "set_favorites",
        "set_preferred_workflow",
        "set_timetracking_enabled",
        "set_show_brackets",
        "set_doc_mode_enter_for_new_block",
        "set_logical_outdenting",
        "set_guide_announced",
        "set_default_journal_template",
        "set_start_of_week",
        "set_preferred_format",
        "set_journal_title_format",
        "read_custom_css",
        "read_asset",
        "stream_asset_path",
        "tine_quit",
        "close_graph_window",
        "tine_open_devtools",
        "read_local_image",
        "import_asset",
        "import_native_capture",
        "read_text_file",
        "detect_media_editor",
        "trash_asset",
        "empty_asset_trash",
        "sync_conflict_diff",
        "resolve_sync_conflict",
        "trash_sync_conflict",
        "trash_journal_file",
        "read_journal_file",
        "save_asset",
        "read_highlights",
        "save_pdf_area_image",
    ];
    for (name, asynchronous, _) in &listed {
        if !asynchronous {
            assert!(AUDITED_SYNC.contains(&name.as_str()),
                "I-13 / OG-RULES Rule 4: new synchronous Tauri command {name} needs a load-wait audit; default to async + blocking pool");
        }
    }
    for name in [
        "list_templates",
        "journal_content_days",
        "resolve_block",
        "resolve_blocks",
        "preview_block",
        "delete_page",
        "merge_pages",
        "rename_file_to_page",
        "copy_guide_into_graph",
        "page_print_html",
        "capture_quick_switch",
        "open_pdf",
        "write_highlights",
        "open_page_file",
        "get_page_by_path",
    ] {
        assert!(
            listed
                .iter()
                .any(|(found, asynchronous, _)| found == name && *asynchronous),
            "I-13 / OG-RULES Rule 4: {name} must stay an asynchronous load-waiting Tauri command"
        );
    }
}

#[test]
fn guard_detects_a_new_synchronous_load_wait() {
    let planted = "#[tauri::command]\npub(crate) fn new_read() -> () { slot.store.whole_graph(); }";
    assert!(commands(planted)
        .iter()
        .any(|(_, asynchronous, body)| !asynchronous && reaches_load_wait(body)));
}

/// What starts a child process: a direct spawn, or a helper that spawns one.
/// Spawning searches PATH and execs (the parent blocks until the child has
/// exec'd), and `output()`/`status()` wait for the child to exit.
fn runs_child_process(body: &str) -> bool {
    [
        "Command::new(",
        "opener_command(",
        "opener_command_env(",
        "open_page_source(",
        "reveal_page_source(",
        "discover_dictionaries",
        "linux_copy_image(",
        "open_asset_with_os(",
        "edit_asset_with_os(",
    ]
    .iter()
    .any(|marker| body.contains(marker))
}

fn tauri_sources(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            tauri_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && !path.to_string_lossy().contains("test")
        {
            let source = std::fs::read_to_string(&path).unwrap();
            if source.contains("#[tauri::command]") {
                out.push((path.display().to_string(), source));
            }
        }
    }
}

#[test]
fn child_process_tauri_commands_leave_the_ui_thread() {
    let mut sources = Vec::new();
    tauri_sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    assert!(sources.len() > 5, "the command sources were found");
    let mut offenders = Vec::new();
    let mut checked = 0;
    for (file, source) in &sources {
        for (name, asynchronous, body) in commands(source) {
            if runs_child_process(&body) {
                checked += 1;
                if !asynchronous || !body.contains("spawn_blocking(") {
                    offenders.push(format!("{name} ({file})"));
                }
            }
        }
    }
    assert!(
        checked >= 6,
        "the child-process commands were found ({checked})"
    );
    assert!(
        offenders.is_empty(),
        "I-21 / GH #623: a synchronous Tauri command runs on the UI thread, and every other \
         synchronous command queues behind it; starting a child process there (PATH search, exec, \
         waiting for its exit) froze every IPC call at launch for 5.4 s while \
         list_spellcheck_dictionaries ran enchant-lsmod. Make it async and start the child on the \
         blocking pool; exemplar spellcheck.rs::list_spellcheck_dictionaries. Offenders: {offenders:?}"
    );
}

#[test]
fn guard_detects_a_synchronous_child_process_command() {
    let planted = "#[tauri::command]\npub(crate) fn probe() -> () { std::process::Command::new(\"x\").output(); }";
    assert!(commands(planted)
        .iter()
        .any(|(_, asynchronous, body)| !asynchronous && runs_child_process(body)));
}
