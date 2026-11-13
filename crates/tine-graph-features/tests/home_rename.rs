//! Family 16/11: renaming the home page moves `:default-home` with it, in the
//! rename's config step: one guarded config.edn transaction after the page
//! operation, which moves `:favorites` too (OG `rename-page-aux`,
//! page.cljs:491 at 6e7afa8eb; CM step 2 batch 4). `merge-pages!` does not
//! move the home, so a merge keeps it. A malformed config.edn never blocks the
//! rename.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tine_graph_features::pages;
use tine_store::{FaultPoint, Store};

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

fn scratch(label: &str, config: &str) -> PathBuf {
    let dir = crate::test_support::fresh_dir(&format!("tine-home-rename-{label}-"));
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(dir.join("logseq/config.edn"), config).unwrap();
    dir
}

fn open(dir: &Path) -> std::sync::Arc<Store> {
    let store = std::sync::Arc::new(Store::open(dir, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    store
}

fn config(dir: &Path) -> String {
    fs::read_to_string(dir.join("logseq/config.edn")).unwrap()
}

const HOME: &str = ";; my settings\n{:file/name-format :triple-lowbar\n :default-home {:page \"Start\" :sidebar [\"Contents\"]}\n :unknown/key #{1 2}}\n";

#[test]
fn renaming_the_home_page_moves_default_home_with_it() {
    let _integration_guard = crate::integration_lock();
    let dir = scratch("primary", HOME);
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    fs::write(dir.join("pages/Ref.md"), "- see [[Start]]\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Start", "Begin", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page.as_deref(), Some("Begin"));
    assert_eq!(config(&dir), HOME.replace("\"Start\"", "\"Begin\""));
    assert_eq!(store.config().config.default_home.as_deref(), Some("Begin"));
    assert_eq!(
        fs::read_to_string(dir.join("pages/Ref.md")).unwrap(),
        "- see [[Begin]]\n"
    );
}

/// OG `page-name-sanity-lc` comparison: the configured name matches the page
/// case-insensitively; namespace descendants follow their parent's rename.
#[test]
fn a_home_named_by_case_or_as_a_namespace_child_follows_the_rename() {
    let _integration_guard = crate::integration_lock();
    let dir = scratch("case", &HOME.replace("\"Start\"", "\"start\""));
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Start", "Begin", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page.as_deref(), Some("Begin"));
    assert_eq!(store.config().config.default_home.as_deref(), Some("Begin"));

    let dir = scratch("namespace", &HOME.replace("\"Start\"", "\"Work/Log\""));
    fs::write(dir.join("pages/Work.md"), "- parent\n").unwrap();
    fs::write(dir.join("pages/Work___Log.md"), "- child\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Work", "Job", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page.as_deref(), Some("Job/Log"));
    assert_eq!(
        store.config().config.default_home.as_deref(),
        Some("Job/Log")
    );
    assert!(dir.join("pages/Job___Log.md").exists());
}

#[test]
fn renaming_another_page_leaves_config_untouched() {
    let _integration_guard = crate::integration_lock();
    let dir = scratch("other", HOME);
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    fs::write(dir.join("pages/Starter.md"), "- other\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Starter", "Kit", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page, None);
    assert_eq!(config(&dir), HOME);
}

/// OG `merge-pages!` leaves `:default-home` alone.
#[test]
fn merging_the_home_page_into_another_keeps_default_home() {
    let _integration_guard = crate::integration_lock();
    let dir = scratch("merge", HOME);
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    fs::write(dir.join("pages/Other.md"), "- other body\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(
            &store,
            host,
            "Start",
            "Other",
            None,
            Some("pages/Other.md"),
            &[],
        )
    })
    .unwrap();
    assert_eq!(report.home_page, None);
    assert_eq!(config(&dir), HOME);
}

/// Scenario (external-editor race / sync delivery): a truncated config.edn is
/// not rewritten by the rename, and does not stop it.
#[test]
fn a_malformed_config_neither_blocks_the_rename_nor_is_rewritten() {
    let _integration_guard = crate::integration_lock();
    let truncated = "{:default-home {:page \"Start\"";
    let dir = scratch("malformed", truncated);
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Start", "Begin", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page, None);
    assert_eq!(report.favorites, None);
    assert_eq!(config(&dir), truncated);
    assert!(dir.join("pages/Begin.md").exists());
}

/// Verification of card HFmqtY (b), scenario: sync delivery of a config.edn
/// that is not UTF-8. The rename does NOT proceed: the Store refuses every
/// write while the config cannot be read (`PagePlan::config_write_failure`),
/// because the graph's `:file/name-format` is then unknown and a page file
/// would be written under the default name. The refusal comes before the
/// config step and before any file moves, so nothing changes: no page file, no
/// referrer, not config.edn's bytes. Pinned so a change to that guard is a
/// decision (see RECEIPT-small-1109c.md), not an accident.
#[test]
fn a_non_utf8_config_refuses_the_rename_before_anything_changes() {
    let _integration_guard = crate::integration_lock();
    let dir = scratch("non-utf8", "");
    let bytes = b"{:default-home {:page \"Start\"}} ;; \xff\xfe\n".to_vec();
    fs::write(dir.join("logseq/config.edn"), &bytes).unwrap();
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    fs::write(dir.join("pages/Ref.md"), "- see [[Start]]\n").unwrap();
    let store = open(&dir);
    let error = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Start", "Begin", None, None, &[])
    })
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{error}");
    // Card HVgII: the refusal reaches the window as the wire family
    // `config-unreadable` (which the window words from its catalog), with the
    // read error after it for logs, never as the bare read error.
    assert!(
        error
            .get_ref()
            .is_some_and(|cause| cause.is::<tine_graph_features::ConfigUnreadable>()),
        "the refusal's type survives: {error}"
    );
    assert!(
        error.to_string().starts_with("config-unreadable: "),
        "{error}"
    );
    assert!(
        pages::RenameReport::applied_before_error(&error).is_none(),
        "refused before any page file was published"
    );
    assert!(dir.join("pages/Start.md").exists());
    assert!(!dir.join("pages/Begin.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join("pages/Ref.md")).unwrap(),
        "- see [[Start]]\n"
    );
    assert_eq!(fs::read(dir.join("logseq/config.edn")).unwrap(), bytes);
}

/// Scenario (disk error): the page files are renamed and published, then the
/// config.edn write fails. The CLI keeps the error (`applied_before_error`:
/// uncertain, admitted); the app's rename turns it into the applied report with
/// the cause in `config_skipped`, so the window refreshes and says that home and
/// favorites did not follow instead of "Rename failed". Nothing is rolled back
/// and config.edn keeps its old bytes.
#[cfg(unix)]
#[test]
fn a_failed_config_write_after_the_page_files_moved_reports_the_rename() {
    use std::os::unix::fs::PermissionsExt;
    let _integration_guard = crate::integration_lock();
    let dir = scratch("config-write-fails", HOME);
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    fs::write(dir.join("pages/Ref.md"), "- see [[Start]]\n").unwrap();
    // The page operation's trash chain exists and stays writable; only
    // `logseq/` itself refuses the config transaction's new file.
    fs::create_dir_all(dir.join("logseq/.tine-trash/pages")).unwrap();
    let store = open(&dir);
    let logseq = dir.join("logseq");
    fs::set_permissions(&logseq, fs::Permissions::from_mode(0o555)).unwrap();
    let writable = fs::write(logseq.join("probe"), "x").is_ok();
    let result = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Start", "Begin", None, None, &[])
    });
    fs::set_permissions(&logseq, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!writable, "the injected failure needs a non-root user");
    let error = result.unwrap_err();
    let applied =
        pages::RenameReport::applied_before_error(&error).expect("the pages were published");
    assert_eq!(applied.home_page, None);
    assert!(applied.touched.iter().any(|page| page.moved));
    let app = pages::RenameReport::with_config_failure(&error).expect("the app's report");
    assert_eq!(app.touched, applied.touched);
    assert_eq!(app.outcome, applied.outcome);
    assert!(
        app.config_skipped
            .as_deref()
            .is_some_and(|cause| !cause.is_empty()),
        "{app:?}"
    );
    assert!(dir.join("pages/Begin.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join("pages/Ref.md")).unwrap(),
        "- see [[Begin]]\n"
    );
    assert_eq!(config(&dir), HOME);
}

/// The renamed page is the home page and a favorite: one config.edn edit
/// moves both, keeping every other byte (comments, unknown keys, the other
/// favorite), and the report carries both answers.
#[test]
fn home_and_favorites_move_together_in_the_config_step() {
    let _integration_guard = crate::integration_lock();
    let before = HOME.replace(
        " :unknown/key",
        " :favorites [\"Start\" \"Kit\"] ;; pinned\n :unknown/key",
    );
    let dir = scratch("together", &before);
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    fs::write(dir.join("pages/Kit.md"), "- kit\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Start", "Begin", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page.as_deref(), Some("Begin"));
    let favorites = report.favorites.expect("favorites moved");
    assert_eq!(favorites.names, ["Begin", "Kit"]);
    assert_eq!(favorites.page, None);
    assert_eq!(report.config_skipped, None);
    assert_eq!(config(&dir), before.replace("\"Start\"", "\"Begin\""));
}

/// A favorite that only renames onto a name already among the favorites (a
/// reference-only favorite) leaves one entry, in the first one's place.
#[test]
fn a_favorite_renamed_onto_another_favorite_is_deduplicated() {
    let _integration_guard = crate::integration_lock();
    let dir = scratch("dedupe", "{:favorites [\"New\" \"Old\" \"Pinned\"]}\n");
    fs::write(dir.join("pages/Old.md"), "- old\n").unwrap();
    fs::write(dir.join("pages/Pinned.md"), "- see [[New]]\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Old", "New", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.favorites.expect("moved").names, ["New", "Pinned"]);
    assert_eq!(config(&dir), "{:favorites [\"New\" \"Pinned\"]}\n");
}

/// Home and favorites are independent: a rename of a page that is only a
/// favorite moves no home, and one that is only the home moves no favorite.
#[test]
fn home_and_favorites_follow_independently() {
    let _integration_guard = crate::integration_lock();
    let start = "{:default-home {:page \"Start\"} :favorites [\"Kit\"]}\n";
    let dir = scratch("independent", start);
    fs::write(dir.join("pages/Start.md"), "- home\n").unwrap();
    fs::write(dir.join("pages/Kit.md"), "- kit\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Kit", "Gear", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page, None);
    assert_eq!(report.favorites.expect("moved").names, ["Gear"]);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Start", "Begin", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page.as_deref(), Some("Begin"));
    assert_eq!(report.favorites, None);
    assert_eq!(
        config(&dir),
        "{:default-home {:page \"Begin\"} :favorites [\"Gear\"]}\n"
    );
}

#[test]
fn crash_home_rename_worker() {
    let _integration_guard = crate::integration_lock();
    let Ok(root) = std::env::var("TINE_HOME_CRASH_ROOT") else {
        return;
    };
    use tine_store::host_faults::{abort_before, Phase};
    let store = open(Path::new(&root));
    match std::env::var("TINE_HOME_CRASH_POINT").unwrap().as_str() {
        "dst" => abort_before(Phase::PageRename, 0),
        "referrer" => abort_before(Phase::PageRename, 1),
        "trash" => abort_before(Phase::TrashMove, 0),
        "config" => store.inject_fault(FaultPoint::AbortAfterStep(0)),
        _ => unreachable!(),
    }
    let app_data = std::env::var("TINE_HOME_CRASH_APP_DATA").unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, Path::new(&app_data)).unwrap();
    let _ = pages::rename_or_merge_page(&store, &host, "Start", "Begin", None, None, &[]);
    panic!("I-2: home rename fault did not abort; exemplar pages::rename_page_expected");
}

/// I-2, restated for the page host (STEP3-DESIGN's restated rollback tests,
/// SPEC-s3 s3.2): the rename is one host operation (destination, then the
/// referrer, then the source's deletion while Tine runs), and `config.edn`'s
/// home moves in its own transaction after it completes. Killed at each
/// boundary: every file is whole old or new bytes, the config parses and is
/// new only once the operation completed, and the home names a live page
/// (the source stays live until the destination and the referrer
/// published). A relaunch on the same app data completes the operation from
/// its drafts; a home the kill left on the old name stays there, as a kill
/// between the old transaction's move and config steps left it.
#[test]
fn a_home_rename_killed_at_each_step_reopens_whole() {
    let _integration_guard = crate::integration_lock();
    let new = HOME.replace("\"Start\"", "\"Begin\"");
    for point in ["dst", "referrer", "trash", "config"] {
        let dir = scratch("crash", HOME);
        let app_data = tempfile::tempdir().unwrap();
        fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
        fs::write(dir.join("pages/Ref.md"), "- see [[Start]]\n").unwrap();
        let output = Command::new(tine_store::test_binary::own_test_binary())
            .args([
                "--exact",
                "home_rename::crash_home_rename_worker",
                "--nocapture",
            ])
            .env("TINE_HOME_CRASH_ROOT", &dir)
            .env("TINE_HOME_CRASH_POINT", point)
            .env("TINE_HOME_CRASH_APP_DATA", app_data.path())
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "I-2: child must abort at {point}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let reopened = open(&dir);
        let text = config(&dir);
        assert!(
            text == HOME || text == new,
            "I-2: config.edn whole at {point}:\n{text}"
        );
        assert_eq!(text == new, point == "config", "config is the last step");
        let reference = fs::read_to_string(dir.join("pages/Ref.md")).unwrap();
        assert!(reference == "- see [[Start]]\n" || reference == "- see [[Begin]]\n");
        for page in ["Start", "Begin"] {
            if let Ok(body) = fs::read_to_string(dir.join(format!("pages/{page}.md"))) {
                assert_eq!(body, "- home body\n", "I-2: whole at {point}");
            }
        }
        let home = reopened.config().config.default_home.clone().unwrap();
        assert!(
            dir.join(format!("pages/{home}.md")).exists(),
            "I-2: home names a live page at {point}"
        );
        // Relaunch: the operation completes from its drafts.
        let host = tine_store::PageHost::start_for_tests(&reopened, app_data.path()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while dir.join("pages/Start.md").exists()
            || fs::read_to_string(dir.join("pages/Ref.md")).unwrap() != "- see [[Begin]]\n"
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the relaunch never completed at {point}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            fs::read_to_string(dir.join("pages/Begin.md")).unwrap(),
            "- home body\n",
            "I-2: home page content kept at {point}"
        );
        assert!(config(&dir) == HOME || config(&dir) == new);
        drop(host);
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }
}
