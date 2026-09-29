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

fn open(dir: &Path) -> Store {
    let store = Store::open(dir, Default::default()).unwrap().0;
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
    let report = pages::rename_or_merge_page(&store, "Start", "Begin", None, None, &[]).unwrap();
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
    let report = pages::rename_or_merge_page(&store, "Start", "Begin", None, None, &[]).unwrap();
    assert_eq!(report.home_page.as_deref(), Some("Begin"));
    assert_eq!(store.config().config.default_home.as_deref(), Some("Begin"));

    let dir = scratch("namespace", &HOME.replace("\"Start\"", "\"Work/Log\""));
    fs::write(dir.join("pages/Work.md"), "- parent\n").unwrap();
    fs::write(dir.join("pages/Work___Log.md"), "- child\n").unwrap();
    let store = open(&dir);
    let report = pages::rename_or_merge_page(&store, "Work", "Job", None, None, &[]).unwrap();
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
    let report = pages::rename_or_merge_page(&store, "Starter", "Kit", None, None, &[]).unwrap();
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
    let report =
        pages::rename_or_merge_page(&store, "Start", "Other", None, Some("pages/Other.md"), &[])
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
    let report = pages::rename_or_merge_page(&store, "Start", "Begin", None, None, &[]).unwrap();
    assert_eq!(report.home_page, None);
    assert_eq!(config(&dir), truncated);
    assert!(dir.join("pages/Begin.md").exists());
}

#[test]
fn crash_home_rename_worker() {
    let Ok(root) = std::env::var("TINE_HOME_CRASH_ROOT") else {
        return;
    };
    let boundary: usize = std::env::var("TINE_HOME_CRASH_BOUNDARY")
        .unwrap()
        .parse()
        .unwrap();
    let store = open(Path::new(&root));
    store.inject_fault(FaultPoint::AbortAfterStep(boundary));
    let _ = pages::rename_or_merge_page(&store, "Start", "Begin", None, None, &[]);
    panic!("I-2: home rename fault did not abort; exemplar pages::rename_page_expected");
}

/// I-2: steps are the referrer rewrite (0), the page move (1) and config.edn
/// (2). At every boundary each file is whole old or new bytes, the config
/// parses, and home names a page that is live or is the renamed one.
#[test]
fn a_home_rename_killed_at_each_step_reopens_whole() {
    for boundary in 0..3 {
        let dir = scratch("crash", HOME);
        fs::write(dir.join("pages/Start.md"), "- home body\n").unwrap();
        fs::write(dir.join("pages/Ref.md"), "- see [[Start]]\n").unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_home_rename_worker", "--nocapture"])
            .env("TINE_HOME_CRASH_ROOT", &dir)
            .env("TINE_HOME_CRASH_BOUNDARY", boundary.to_string())
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "I-2: child must abort after step {boundary}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let reopened = open(&dir);
        let text = config(&dir);
        let new = HOME.replace("\"Start\"", "\"Begin\"");
        assert!(
            text == HOME || text == new,
            "I-2: config.edn whole at step {boundary}:\n{text}"
        );
        assert_eq!(text == new, boundary == 2, "config is the last step");
        let reference = fs::read_to_string(dir.join("pages/Ref.md")).unwrap();
        assert!(reference == "- see [[Start]]\n" || reference == "- see [[Begin]]\n");
        let live = if dir.join("pages/Begin.md").exists() {
            "Begin"
        } else {
            "Start"
        };
        assert_eq!(
            fs::read_to_string(dir.join(format!("pages/{live}.md"))).unwrap(),
            "- home body\n",
            "I-2: home page content kept at step {boundary}"
        );
        let home = reopened.config().config.default_home.clone().unwrap();
        assert!(
            home == live || boundary < 2,
            "home names the live page once the rename completed"
        );
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }
}
