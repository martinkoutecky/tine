//! CM step 2, slice 2, item 3: O8, the settings step (model r6,
//! `tine-agents/specs/notes/cm-cut-model-r6.md`; Lean `CutSettings`,
//! `settingsAfter`, `finishSettings`). "After the files: Favorites entries
//! follow O1; after a merge the list then keeps only the first entry of each
//! name. The Favorites-page setting follows. The home page follows a rename
//! and stays on a merge. The settings file is written in its own step after
//! the pages, all of it or none." Each test renames through the app's entry
//! and asserts config.edn on disk and the report the window adopts.
use super::*;

fn with_config(label: &str, config: &str, files: &[(&str, &str)]) -> (PathBuf, Arc<Store>) {
    let root = crate::test_support::fresh_dir(&format!("tine-cm-settings-{label}-"));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("logseq/config.edn"), config).unwrap();
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (root, store)
}

fn renamed(
    store: &Arc<Store>,
    old: &str,
    new: &str,
    choice: RenameChoice<'_>,
) -> tine_graph_features::pages::RenameReport {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    rename_page_as(store, &host, old, new, None, choice, &[]).unwrap()
}

fn config(root: &Path) -> String {
    fs::read_to_string(root.join("logseq/config.edn")).unwrap()
}

#[cfg(unix)]
#[test]
fn o8_a_failed_step_keeps_the_whole_old_settings_record_and_reports_pages_done() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = crate::integration_lock();
    let settings = "{:default-home {:page \"A\"} :favorites [\"A\" \"A/child\"] :tine/favorites-page \"A/layout\"}\n";
    let (root, store) = with_config("failed-record", settings, &[("pages/A.md", "- source\n")]);
    fs::create_dir_all(root.join("logseq/.tine-trash/pages")).unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let directory = root.join("logseq");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    let result = rename_page_as(&store, &host, "A", "B", None, RenameChoice::Rename, &[]);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    let error = result.unwrap_err();
    let applied = tine_graph_features::pages::RenameReport::applied_before_error(&error).unwrap();
    assert_eq!(applied.outcome, RenameOutcome::Renamed);
    let shown = tine_graph_features::pages::RenameReport::with_config_failure(&error).unwrap();
    assert!(shown
        .config_skipped
        .as_ref()
        .is_some_and(|why| !why.is_empty()));
    assert_eq!(shown.home_page, None);
    assert_eq!(shown.favorites, None);
    assert_eq!(config(&root), settings);
    assert!(!root.join("pages/A.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("pages/B.md")).unwrap(),
        "- source\n"
    );
}

/// O8 (Lean `CutSettingsExamples.favorites_first_name`, `home_merge_stays`):
/// "after a merge the list then keeps only the first entry of each name".
/// Favorites `A, X, B`, merge A into B: A's entry becomes B in A's place and
/// B's own later entry goes, giving `B, X`; the home page `A` stays on a
/// merge.
#[test]
fn o8_after_a_merge_each_page_is_listed_once_and_the_first_entry_stays() {
    let (root, store) = with_config(
        "merge-first",
        "{:default-home {:page \"A\"} :favorites [\"A\" \"X\" \"B\"]}\n",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/B.md", "- b\n"),
            ("pages/X.md", "- x\n"),
        ],
    );
    let report = renamed(&store, "A", "B", RenameChoice::MergeInto("pages/B.md"));
    assert_eq!(report.outcome, RenameOutcome::Merged);
    assert_eq!(report.favorites.expect("favorites moved").names, ["B", "X"]);
    assert_eq!(report.home_page, None);
    assert_eq!(
        config(&root),
        "{:default-home {:page \"A\"} :favorites [\"B\" \"X\"]}\n"
    );
}

/// O8 follows every successful rename (Lean `finishSettings` consumes the
/// page result, not a count of changed bytes); review round 2, finding 2,
/// with the reviewer's input. `Old` has no file and is named only by an Org
/// tag, which cannot hold `two words` (O7: left and reported), so no page
/// bytes change; the rename still completes and the settings follow. The
/// same-spelling request is the control: it writes nothing.
#[test]
fn o8_a_rename_that_changes_no_page_bytes_still_moves_the_settings() {
    let settings = "{:default-home {:page \"Old\"} :favorites [\"Old\"]}\n";
    let tagged = "* heading :Old:\n";
    let (root, store) = with_config("no-bytes", settings, &[("pages/r.org", tagged)]);
    let report = renamed(&store, "Old", "two words", RenameChoice::Rename);
    assert_eq!(report.outcome, RenameOutcome::Renamed);
    assert_eq!(
        report.site_exceptions.len(),
        1,
        "{:?}",
        report.site_exceptions
    );
    assert_eq!(report.site_exceptions[0].path, "pages/r.org");
    assert_eq!(report.config_skipped, None);
    assert_eq!(report.home_page.as_deref(), Some("two words"));
    assert_eq!(
        report.favorites.expect("favorites moved").names,
        ["two words"]
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/r.org")).unwrap(),
        tagged
    );
    assert_eq!(
        config(&root),
        "{:default-home {:page \"two words\"} :favorites [\"two words\"]}\n"
    );

    let (root, store) = with_config("same-spelling", settings, &[("pages/r.org", tagged)]);
    let report = renamed(&store, "Old", "Old", RenameChoice::Rename);
    assert_eq!(report.outcome, RenameOutcome::Unchanged);
    assert_eq!(
        config(&root),
        settings,
        "a same-spelling request writes nothing"
    );
}

/// O8 with O1: "Favorites entries follow O1", and O1 substitutes the
/// namespace prefix for every name under the source "however it exists": a
/// favorite or home page naming a reference-only child (`A/x`, only linked)
/// or a name under A that exists only in the settings (`A/y`) follows to
/// `B/…` like a file-backed child.
#[test]
fn o8_settings_under_the_source_follow_by_namespace_prefix() {
    let (root, store) = with_config(
        "prefix",
        "{:default-home {:page \"A/x\"} :favorites [\"A/x\" \"A/y\" \"Other\"]}\n",
        &[("pages/A.md", "- a\n"), ("pages/Other.md", "- [[A/x]]\n")],
    );
    let report = renamed(&store, "A", "B", RenameChoice::Rename);
    assert_eq!(report.home_page.as_deref(), Some("B/x"));
    assert_eq!(
        report.favorites.expect("favorites moved").names,
        ["B/x", "B/y", "Other"]
    );
    assert_eq!(
        config(&root),
        "{:default-home {:page \"B/x\"} :favorites [\"B/x\" \"B/y\" \"Other\"]}\n"
    );
}

/// O8: "after a merge the list then keeps only the first entry of each name"
/// covers the whole list (Lean `favoritesAfter` applies `uniqueNames` to
/// every entry): a repeated favorite the merge did not move keeps its first
/// entry too, and a day is one name in all its spellings.
#[test]
fn o8_after_a_merge_every_repeated_favorite_is_listed_once() {
    let (root, store) = with_config(
        "merge-repeats",
        "{:favorites [\"X\" \"x\" \"Nov 2nd, 2026\" \"2026_11_02\"]}\n",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/B.md", "- b\n"),
            ("pages/X.md", "- x\n"),
        ],
    );
    let report = renamed(&store, "A", "B", RenameChoice::MergeInto("pages/B.md"));
    assert_eq!(
        report.favorites.expect("favorites changed").names,
        ["X", "Nov 2nd, 2026"]
    );
    assert_eq!(config(&root), "{:favorites [\"X\" \"Nov 2nd, 2026\"]}\n");
}

// The plain-rename half of O8's one-entry rule (no entry dropped) is
// `home_rename::a_favorite_renamed_onto_another_favorite_keeps_both_entries`.
