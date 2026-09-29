//! Rule 8 census of Tauri page-content writers and their kind-taking store path.

fn body<'a>(source: &'a str, name: &str) -> &'a str {
    let signature = format!("fn {name}(");
    let start = source
        .find(&signature)
        .unwrap_or_else(|| panic!("missing writer {name}"));
    let open = source[start..].find('{').unwrap() + start;
    let mut depth = 0;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        if *byte == b'{' {
            depth += 1;
        }
        if *byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return &source[open..open + offset + 1];
            }
        }
    }
    panic!("unclosed writer {name}")
}

/// Whitespace-insensitive containment, so `cargo fmt` line wrapping cannot
/// break a census that is about calls, not layout.
fn calls(body: &str, marker: &str) -> bool {
    let squash = |text: &str| text.split_whitespace().collect::<String>();
    squash(body).contains(&squash(marker))
}

#[test]
fn every_tauri_page_writer_reaches_a_kind_taking_store_entry() {
    const COMMANDS: &str = include_str!("commands.rs");
    const CONCORD: &str = include_str!("commands/concord.rs");
    const BACKUP: &str = include_str!("backup/restore.rs");
    const PAGES: &str = include_str!("../../crates/tine-graph-features/src/pages.rs");
    const CONFLICTS: &str = include_str!("../../crates/tine-graph-features/src/conflicts.rs");
    const LIVE: &str = include_str!("../../crates/tine-graph-features/src/live_conflict.rs");
    const PDF: &str = include_str!("../../crates/tine-graph-features/src/pdf.rs");
    const GUIDE: &str = include_str!("../../crates/tine-graph-features/src/guide.rs");
    const JOURNALS: &str = include_str!("../../crates/tine-graph-features/src/journals.rs");
    const FEATURES: &str = include_str!("../../crates/tine-graph-features/src/lib.rs");
    let routes = [
        ("save_pages", "tine_graph_features::pages::save_pages"),
        (
            "delete_page",
            "tine_graph_features::pages::delete_page_expected",
        ),
        (
            "rename_page",
            "tine_graph_features::pages::rename_or_merge_page",
        ),
        (
            "copy_guide_into_graph",
            "tine_graph_features::guide::copy_guide_into_graph",
        ),
        (
            "set_journal_title_format",
            "tine_graph_features::config::set_journal_page_title_format",
        ),
        (
            "trash_journal_file",
            "tine_graph_features::journals::trash_journal_file",
        ),
        ("merge_pages", "tine_graph_features::pages::merge_pages"),
        (
            "rename_file_to_page",
            "tine_graph_features::pages::rename_file_to_page",
        ),
        (
            "write_highlights",
            "tine_graph_features::pdf::write_highlights",
        ),
        ("open_pdf", "tine_graph_features::pdf::open_pdf"),
    ];
    // Concord's writers live in their own command module (og family 8).
    let concord_routes = [
        (
            "resolve_sync_conflict",
            "tine_graph_features::conflicts::resolve_sync_conflict",
        ),
        (
            "trash_sync_conflict",
            "tine_graph_features::conflicts::trash_sync_conflict",
        ),
        (
            "resolve_vcs_marker_conflict",
            "tine_graph_features::conflicts::resolve_vcs_marker_conflict",
        ),
        (
            "resolve_live_conflict",
            "tine_graph_features::live_conflict::resolve_live_conflict",
        ),
    ];
    assert_eq!(
        routes.len() + concord_routes.len(),
        14,
        "OG-RULES Rule 8: update the page-writer census; exemplar src-tauri/src/commands.rs"
    );
    for (name, route) in routes {
        assert!(calls(body(COMMANDS, name), route), "OG-RULES Rule 8: {name} changed its page-write route; exemplar src-tauri/src/commands.rs");
    }
    for (name, route) in concord_routes {
        assert!(calls(body(CONCORD, name), route), "OG-RULES Rule 8: {name} changed its page-write route; exemplar src-tauri/src/commands/concord.rs");
    }
    let destinations = [
        (PAGES, "save_pages", "store.save_pages(&prepared)"),
        (
            PAGES,
            "delete_page_expected",
            "transaction(Some(tine_store::EditKind::DeletePage))",
        ),
        (
            PAGES,
            "rename_page_after_inventory",
            "transaction(Some(tine_store::EditKind::RenamePage))",
        ),
        (
            PAGES,
            "rename_file_to_page",
            "transaction(Some(tine_store::EditKind::RenamePage))",
        ),
        (
            PAGES,
            "merge_pages",
            "tx.save_page(&[tine_store::EditKind::InsertBlocks",
        ),
        (
            GUIDE,
            "create_if_absent",
            "transaction(Some(tine_store::EditKind::ReplacePage))",
        ),
        (
            JOURNALS,
            "migrate_journal_filenames",
            "transaction(Some(tine_store::EditKind::RenamePage))",
        ),
        (
            FEATURES,
            "trash_current",
            "transaction(Some(tine_store::EditKind::DeletePage))",
        ),
        (
            CONFLICTS,
            "resolve_sync_conflict",
            "tx.save_page(&[tine_store::EditKind::ReplacePage",
        ),
        (
            CONFLICTS,
            "resolve_vcs_marker_conflict",
            "tx.save_page(&[tine_store::EditKind::ReplacePage], &page, SaveBase::ResolvingMarkers(rev)",
        ),
        (LIVE, "resolve_live_conflict", "tx.save_page(&[kind], &page, base"),
        (
            PDF,
            "write_highlights",
            "tx.save_page(&[tine_store::EditKind::ReplacePage",
        ),
        (
            PDF,
            "open_pdf",
            "transaction(Some(tine_store::EditKind::CreatePage))",
        ),
    ];
    for (source, name, marker) in destinations {
        assert!(calls(body(source, name), marker), "OG-RULES Rule 8: {name} must reach a kind-taking store entry; exemplar crates/tine-graph-features/src/pages.rs");
    }
    assert!(calls(
        body(BACKUP, "restore_backup"),
        "restore_from_backup_source"
    ));
    assert!(calls(
        body(BACKUP, "restore_from_backup_source"),
        ".restore(tine_store::EditKind::ReplacePage"
    ));
}

#[test]
fn writer_guard_rejects_a_missing_kind_path() {
    let altered = "fn delete_page_expected() { store.transaction(None); }";
    assert!(!calls(
        body(altered, "delete_page_expected"),
        "transaction(Some(tine_store::EditKind::DeletePage))"
    ));
}
