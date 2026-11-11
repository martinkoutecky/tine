use super::*;

/// Card HFmqtY 2a at the command boundary: `rename_page` answers the window
/// through `window_answer`, over the same `rename_page_as` call. When the page
/// files moved and only the config.edn write failed, the window gets the
/// applied report (cause in `config_skipped`), not an error string it would
/// show as "Rename failed". Without the conversion this returns `Err` and fails.
#[cfg(unix)]
#[test]
fn a_failed_config_write_after_the_files_moved_reaches_the_window_as_the_rename() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    // The page operation's trash chain exists and stays writable; only
    // `logseq/` itself refuses the config transaction's new file.
    std::fs::create_dir_all(dir.join("logseq/.tine-trash/pages")).unwrap();
    let config = "{:file/name-format :triple-lowbar\n :default-home {:page \"Start\"}}\n";
    std::fs::write(dir.join("logseq/config.edn"), config).unwrap();
    std::fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    std::fs::write(dir.join("pages/Ref.md"), "- see [[Start]]\n").unwrap();
    let store = std::sync::Arc::new(tine_store::Store::open(dir, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let logseq = dir.join("logseq");
    std::fs::set_permissions(&logseq, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::write(logseq.join("probe"), "x").is_ok();
    let answer = window_answer(tine_graph_features::pages::rename_page_as(
        &store,
        &host,
        "Start",
        "Begin",
        None,
        tine_graph_features::pages::RenameChoice::Rename,
        &[],
    ));
    std::fs::set_permissions(&logseq, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!writable, "the injected failure needs a non-root user");
    let report = answer.expect("the window is told the rename happened");
    assert_eq!(
        report.outcome,
        tine_graph_features::pages::RenameOutcome::Renamed
    );
    assert!(
        report
            .config_skipped
            .as_deref()
            .is_some_and(|cause| !cause.is_empty()),
        "{report:?}"
    );
    assert_eq!(report.home_page, None);
    assert!(report
        .touched
        .iter()
        .any(|page| page.path == "pages/Start.md" && page.moved));
    assert_eq!(
        report
            .renamed
            .iter()
            .map(|page| (page.from.as_str(), page.to.as_str()))
            .collect::<Vec<_>>(),
        [("Start", "Begin")]
    );
    assert!(dir.join("pages/Begin.md").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("pages/Ref.md")).unwrap(),
        "- see [[Begin]]\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("logseq/config.edn")).unwrap(),
        config
    );
}

/// Card HVgII at the command boundary: a rename refused because config.edn is
/// not UTF-8 reaches the window as the wire family `config-unreadable`, which
/// the window words from its catalog, not as the bare read error ("invalid utf-8
/// sequence of 1 bytes from index 35"). Nothing is renamed.
#[test]
fn a_rename_refused_for_an_unreadable_config_reaches_the_window_as_its_wire_family() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    std::fs::create_dir_all(dir.join("logseq")).unwrap();
    std::fs::write(
        dir.join("logseq/config.edn"),
        b"{:default-home {:page \"Start\"}} ;; \xff\xfe\n",
    )
    .unwrap();
    std::fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    let store = std::sync::Arc::new(tine_store::Store::open(dir, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let refused = window_answer(tine_graph_features::pages::rename_page_as(
        &store,
        &host,
        "Start",
        "Begin",
        None,
        tine_graph_features::pages::RenameChoice::Rename,
        &[],
    ))
    .expect_err("a read-only graph refuses the rename");
    assert!(refused.starts_with("config-unreadable: "), "{refused}");
    assert!(dir.join("pages/Start.md").exists());
    assert!(!dir.join("pages/Begin.md").exists());
}
