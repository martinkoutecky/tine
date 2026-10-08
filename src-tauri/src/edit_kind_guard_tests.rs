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

/// Production part of a source file: everything before its `mod tests`.
fn production(source: &str) -> &str {
    source
        .split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or(source)
}

/// Every graph-writing Store entry in `source` names its edit kind:
/// `transaction(None)` or a kind-less `restore` would write pages outside
/// the Rule 8 census, and `save_pages` belongs to the page-save front door.
fn store_entries_take_a_kind(source: &str) -> Result<(), String> {
    let squash = |text: &str| text.split_whitespace().collect::<String>();
    let source = squash(production(source));
    for (entry, kind_taking) in [
        (".transaction(", ".transaction(Some(tine_store::EditKind::"),
        (".restore(", ".restore(tine_store::EditKind::"),
        (".save_pages(", "\0never"),
    ] {
        let (calls, kinded) = (
            source.matches(entry).count(),
            source.matches(kind_taking).count(),
        );
        if calls != kinded {
            return Err(format!(
                "{entry} called {calls}x, {kinded}x with an edit kind"
            ));
        }
    }
    Ok(())
}

#[test]
fn pdf_open_reads_state_without_a_graph_write() {
    let source = include_str!("../../crates/tine-graph-features/src/pdf.rs");
    let open = body(source, "open_pdf");
    assert!(calls(open, "sidecar(store, pdf_name, true)"));
    assert!(
        !calls(open, "transaction("),
        "I-2: PDF open is read-only; exemplar pdf::open_pdf"
    );
    assert!(
        !calls(open, "page_id("),
        "I-12: annotation creation belongs to write_highlights"
    );
}

#[test]
fn every_tauri_page_writer_reaches_a_kind_taking_store_entry() {
    const COMMANDS: &str = include_str!("commands.rs");
    const CONCORD: &str = include_str!("commands/concord.rs");
    // The backup module is two files since the og-B seam split; scan both so
    // the split never shrinks this census.
    const BACKUP: &str = include_str!("backup/restore.rs");
    const BACKUP_SNAPSHOT: &str = include_str!("backup.rs");
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
        (
            "resolve_duplicate_journal_day",
            "tine_graph_features::conflicts::resolve_duplicate_journal_day",
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
        // Both two-file folds (a sync copy, a duplicate journal day) write
        // through the one shared `fold_pair`.
        (CONFLICTS, "resolve_sync_conflict", "fold_pair("),
        (CONFLICTS, "resolve_duplicate_journal_day", "fold_pair("),
        (
            CONFLICTS,
            "fold_pair",
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
    for (file, source) in [
        ("src-tauri/src/backup.rs", BACKUP_SNAPSHOT),
        ("src-tauri/src/backup/restore.rs", BACKUP),
    ] {
        if let Err(why) = store_entries_take_a_kind(source) {
            panic!("OG-RULES Rule 8: {file}: {why}; exemplar src-tauri/src/backup/restore.rs");
        }
    }
}

#[test]
fn writer_guard_rejects_a_missing_kind_path() {
    let altered = "fn delete_page_expected() { store.transaction(None); }";
    assert!(!calls(
        body(altered, "delete_page_expected"),
        "transaction(Some(tine_store::EditKind::DeletePage))"
    ));
}

#[test]
fn backup_census_rejects_a_writer_without_a_kind() {
    let kindless =
        "fn snapshot_repair() { store.transaction(None).unwrap(); }\n#[cfg(test)]\nmod tests {}";
    assert!(store_entries_take_a_kind(kindless).is_err());
    let restore = "fn r() { store.restore(files, None) }";
    assert!(store_entries_take_a_kind(restore).is_err());
    let front_door = "fn r() { store.save_pages(&prepared) }";
    assert!(store_entries_take_a_kind(front_door).is_err());
    let kinded = "fn r() { store.transaction(Some(tine_store::EditKind::ReplacePage)) }";
    assert!(store_entries_take_a_kind(kinded).is_ok());
    let test_only = "fn r() {}\n#[cfg(test)]\nmod tests { fn t() { store.transaction(None); } }";
    assert!(store_entries_take_a_kind(test_only).is_ok());
}
