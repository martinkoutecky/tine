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
    // Store::whole_graph / refresh / the cold-cache publication wait.
    [
        ".whole_graph(",
        ".refresh(",
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
        "guide_pages",
        "read_asset",
        "stream_asset_path",
        "tine_quit",
        "close_graph_window",
        "tine_open_devtools",
        "read_local_image",
        "read_text_file",
        "detect_media_editor",
        "trash_asset",
        "sync_conflict_diff",
        "read_journal_file",
        "read_highlights",
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
        "save_pages",
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

// --- R3 (og-flow3 finding 3): a command that takes the store writer or fsyncs
// leaves the main thread. -----------------------------------------------------

/// What takes the store writer (a page, config, asset, journal, PDF or
/// conflict write through `tine_store`) or syncs a file or directory, directly
/// or through a crate helper that does. `*_blocking(` names a command's own
/// blocking body (exemplar graph.rs `create_graph`). A marker that starts with
/// an identifier matches only at an identifier boundary (`store_at(` is not
/// `restore_at(`).
const WRITER_OR_FSYNC: &[&str] = &[
    // store writer
    "save_pages_wire(",
    "tine_graph_features::config::set_",
    "tine_graph_features::assets::save_asset(",
    "import_asset_file(",
    "import_asset_from_path(",
    "purge_asset_trash(",
    "tine_graph_features::journals::trash_journal_file(",
    "tine_graph_features::pdf::write_pdf_area_image(",
    "tine_graph_features::pdf::rollback_pdf_area_image(",
    "tine_graph_features::conflicts::resolve_sync_conflict(",
    "tine_graph_features::conflicts::trash_sync_conflict(",
    "create_demo_graph(",
    // fsync (temp + fsync + rename + directory sync helpers)
    "update_settings(",
    "update_settings_strict_at(",
    "set_notice_at(",
    "set_crossing_notice_at(",
    "store_at(",
    "retire_at(",
    "publish_package(",
    "uninstall_package(",
    "set_plugin_enabled_at(",
    "store_plugin_registry_cache_at(",
    "remember_external_assets_approval(",
    "remember_graph(",
    "save_workspaces_at(",
    "crate::settings::save_workspaces(",
    "atomic_write(",
    "atomic_update(",
    "atomic_write_session(",
    "flush_now(",
    "prune_backups(",
    "sync_all(",
    "sync_directory_entry(",
    "_blocking(",
];

fn takes_writer_or_fsyncs(body: &str) -> Option<&'static str> {
    WRITER_OR_FSYNC.iter().copied().find(|marker| {
        let identifier = marker.as_bytes()[0].is_ascii_alphabetic() && !marker.starts_with("_");
        body.match_indices(marker).any(|(at, _)| {
            !identifier
                || at == 0
                || !matches!(body.as_bytes()[at - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
        })
    })
}

const OFF_THREAD: &[&str] = &["off_ui(", "spawn_blocking(", "off_ui_graph_read("];

/// Every `async fn` in the app sources whose own body hands its work to the
/// blocking pool (for example `commands.rs::with_config_store`,
/// `settings.rs::save_workspaces`). A command that awaits one of them leaves
/// the main thread just as if it called `off_ui` itself. One level only: a
/// helper must leave the thread in its own body.
fn off_thread_helpers() -> Vec<String> {
    let mut sources = Vec::new();
    tauri_sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    sources
        .iter()
        .flat_map(|(_, source)| async_fns(source))
        .filter(|(_, body)| OFF_THREAD.iter().any(|helper| body.contains(helper)))
        .map(|(name, _)| name)
        .collect()
}

fn async_fns(source: &str) -> Vec<(String, String)> {
    source
        .split("async fn ")
        .skip(1)
        .filter_map(|tail| {
            let name = tail
                .split_once(|c: char| c == '(' || c == '<')?
                .0
                .trim()
                .to_owned();
            let open = tail.find('{')?;
            if tail[..open].contains(';') {
                return None;
            }
            let mut depth = 0;
            for (offset, byte) in tail.as_bytes()[open..].iter().enumerate() {
                match *byte {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some((name, tail[open..open + offset + 1].to_owned()));
                        }
                    }
                    _ => {}
                }
            }
            None
        })
        .collect()
}

fn calls(body: &str, function: &str) -> bool {
    let call = format!("{function}(");
    body.match_indices(&call).any(|(at, _)| {
        at == 0
            || !matches!(body.as_bytes()[at - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
    })
}

fn leaves_main_thread(body: &str, helpers: &[String]) -> bool {
    OFF_THREAD.iter().any(|helper| body.contains(helper))
        || (body.contains(".await") && helpers.iter().any(|helper| calls(body, helper)))
}

/// The commands made async by og-flow3 (R3). Each was ordered by the main
/// thread; the frontend now issues it through its ordered lane, so this list
/// must equal `ORDERED_COMMANDS` in src/orderedWrites.ts.
const ORDERED_WRITES: &[&str] = &[
    "save_pages",
    "save_workspaces",
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
    "set_favorites",
    "set_default_home",
    "import_asset",
    "import_native_capture",
    "empty_asset_trash",
    "trash_journal_file",
    "save_asset",
    "save_pdf_area_image",
    "rollback_pdf_area_image",
    "resolve_sync_conflict",
    "trash_sync_conflict",
    "store_draft",
    "retire_draft",
    "set_app_bool",
    "set_app_string",
    "set_capture_enter_files",
    "set_smooth_scroll",
    "forget_known_graph",
    "set_backup_keep",
    "set_watch_mode",
    "dismiss_defender_hint",
    "approve_external_assets",
    "create_graph",
    "install_plugin",
    "uninstall_plugin",
    "set_plugin_enabled",
    "store_plugin_registry_cache",
    "diagnostic_session_active",
    "clear_diagnostics",
];

fn all_commands() -> Vec<(String, String, bool, String)> {
    let mut sources = Vec::new();
    tauri_sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    sources
        .iter()
        .flat_map(|(file, source)| {
            commands(source)
                .into_iter()
                .map(move |(name, asynchronous, body)| (file.clone(), name, asynchronous, body))
        })
        .collect()
}

fn writer_offenders(
    commands: &[(String, String, bool, String)],
    helpers: &[String],
) -> Vec<String> {
    commands
        .iter()
        .filter_map(|(file, name, asynchronous, body)| {
            let marker = takes_writer_or_fsyncs(body)?;
            (!asynchronous || !leaves_main_thread(body, helpers))
                .then(|| format!("{name} ({file}, via {marker})"))
        })
        .collect()
}

#[test]
fn writer_and_fsync_tauri_commands_leave_the_ui_thread() {
    let commands = all_commands();
    assert!(
        commands.len() > 100,
        "the command sources were found ({})",
        commands.len()
    );
    let checked = commands
        .iter()
        .filter(|(_, _, _, body)| takes_writer_or_fsyncs(body).is_some())
        .count();
    assert!(
        checked >= ORDERED_WRITES.len(),
        "the writer/fsync commands were found ({checked})"
    );
    let helpers = off_thread_helpers();
    let offenders = writer_offenders(&commands, &helpers);
    assert!(
        offenders.is_empty(),
        "R3 / I-21: a synchronous Tauri command runs on the main thread, and every other \
         synchronous command queues behind it. Taking the store writer waits behind a watcher \
         cycle or a checkpoint capture, and an fsync took 2-3 s on a slow Windows disk (GH #623). \
         Make it async and run the write through crate::state::off_ui; exemplar \
         commands.rs::save_pages. If it was ordered by the main thread, add it to ORDERED_WRITES \
         here and ORDERED_COMMANDS in src/orderedWrites.ts. Offenders: {offenders:?}"
    );
    for exemplar in ["with_config_store", "save_workspaces"] {
        assert!(
            helpers.iter().any(|h| h == exemplar),
            "off-thread helper {exemplar} was found: {helpers:?}"
        );
    }
    for name in ORDERED_WRITES {
        assert!(
            commands
                .iter()
                .any(|(_, found, asynchronous, body)| found == name
                    && *asynchronous
                    && leaves_main_thread(body, &helpers)),
            "R3: {name} must stay an async command that runs its write off the main thread"
        );
    }
}

#[test]
fn ordered_writes_match_the_frontend_ordered_lane() {
    let lane = include_str!("../../src/orderedWrites.ts");
    let start = lane
        .find("const ORDERED_COMMANDS")
        .expect("src/orderedWrites.ts declares ORDERED_COMMANDS");
    let block = &lane[start..start + lane[start..].find("]);").expect("ORDERED_COMMANDS ends")];
    let listed: std::collections::BTreeSet<&str> = block.split('"').skip(1).step_by(2).collect();
    let expected: std::collections::BTreeSet<&str> = ORDERED_WRITES.iter().copied().collect();
    assert_eq!(
        listed, expected,
        "R3: every write command moved off the main thread keeps the main thread's issue order \
         through the frontend's ordered lane; ORDERED_WRITES (here) and ORDERED_COMMANDS \
         (src/orderedWrites.ts) must name the same commands"
    );
}

#[test]
fn guard_detects_a_synchronous_writer_or_fsync_command() {
    let planted = [
        (
            "x.rs".to_string(),
            "save_page".to_string(),
            false,
            "{ save_wire::save_pages_wire(&slot.store, &e, f) }".to_string(),
        ),
        (
            "x.rs".to_string(),
            "set_flag".to_string(),
            false,
            "{ update_settings(&app, |j| ()) }".to_string(),
        ),
        (
            "x.rs".to_string(),
            "lazy".to_string(),
            true,
            "{ crate::settings::set_notice_at(&d, &r, k, true) }".to_string(),
        ),
        (
            "x.rs".to_string(),
            "ok".to_string(),
            true,
            "{ crate::state::off_ui(move || store_at(&p, r)).await }".to_string(),
        ),
        (
            "x.rs".to_string(),
            "restore".to_string(),
            false,
            "{ restore_at(&p) }".to_string(),
        ),
        // Awaiting an off-thread helper counts; naming it in a sync body does not,
        // and neither does a helper-named suffix of another function.
        (
            "x.rs".to_string(),
            "ok_helper".to_string(),
            true,
            "{ with_config_store(&s, move |st| tine_graph_features::config::set_x(st)).await }"
                .to_string(),
        ),
        (
            "x.rs".to_string(),
            "sync_helper".to_string(),
            false,
            "{ with_config_store(&s, move |st| tine_graph_features::config::set_x(st)) }"
                .to_string(),
        ),
        (
            "x.rs".to_string(),
            "suffix".to_string(),
            true,
            "{ not_with_config_store(&s, |st| tine_graph_features::config::set_x(st)).await }"
                .to_string(),
        ),
    ];
    let helpers = vec!["with_config_store".to_string()];
    let offenders = writer_offenders(&planted, &helpers);
    assert_eq!(offenders.len(), 5, "{offenders:?}");
    assert!(offenders.iter().all(|offender| {
        !offender.starts_with("ok ")
            && !offender.starts_with("restore ")
            && !offender.starts_with("ok_helper ")
    }));
}
