//! Family 16/11: renaming the home page moves `:default-home` with it, in the
//! rename's own guarded transaction (OG `rename-page-aux`, page.cljs:491 at
//! 6e7afa8eb). `merge-pages!` does not, so a merge keeps it. A malformed
//! config.edn never blocks the rename.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

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
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tine-home-rename-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
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
    let truncated = "{:default-home {:page \"Start\"";
    let dir = scratch("malformed", truncated);
    fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
    let store = open(&dir);
    let report = hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Start", "Begin", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.home_page, None);
    assert_eq!(config(&dir), truncated);
    assert!(dir.join("pages/Begin.md").exists());
}

#[test]
fn crash_home_rename_worker() {
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
    let new = HOME.replace("\"Start\"", "\"Begin\"");
    for point in ["dst", "referrer", "trash", "config"] {
        let dir = scratch("crash", HOME);
        let app_data = tempfile::tempdir().unwrap();
        fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
        fs::write(dir.join("pages/Ref.md"), "- see [[Start]]\n").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_home_rename_worker", "--nocapture"])
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
