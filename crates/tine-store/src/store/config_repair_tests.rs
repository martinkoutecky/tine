//! A repaired config is noticed without reopening
//! (`docs/contracts/config-live-reload.md`): a graph opened while a `logseq`
//! file blocks its config read is read-only, and recovers editing on the next
//! reload or an explicit refresh once the file goes, whether nothing, a
//! directory without a config or a directory with one takes its place. The
//! blocked and the repaired reads both have no config revision, so only the
//! read's outcome tells them apart.
use super::memory_replace_tests::{apply, open, Files};
use super::{Depth, OpenOptions, Store, WatchMode};
use crate::transaction::config_write_failure;
use crate::PageId;
use std::path::Path;
use std::time::{Duration, Instant};
use tine_core::model::Format;

/// The repairs, each as the files it removes and writes, in order (a memory
/// batch neither passes through nor replaces a key, so the file goes first).
type Repair = (
    &'static str,
    &'static [(&'static str, Option<&'static str>)],
);

const REPAIRS: [Repair; 3] = [
    ("file removed", &[("logseq", None)]),
    (
        "a directory with no config",
        &[("logseq", None), ("logseq/custom.css", Some("a {}\n"))],
    ),
    (
        "a directory with a config",
        &[
            ("logseq", None),
            ("logseq/config.edn", Some("{:preferred-format \"Org\"}\n")),
        ],
    ),
];

fn blocked_disk_graph(root: &Path) {
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(root.join("pages/a.md"), "- a\n").unwrap();
    std::fs::write(root.join("logseq"), "x").unwrap();
}

fn repair_disk(root: &Path, steps: &[(&str, Option<&str>)]) {
    for (rel, text) in steps {
        let path = root.join(rel);
        match text {
            None => std::fs::remove_file(path).unwrap(),
            Some(text) => {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
        }
    }
}

fn open_disk(root: &Path) -> Store {
    let options = OpenOptions {
        approved_external_assets: None,
        watch: WatchMode::Poll,
        launch_checkpoint: None,
    };
    let (store, _, state) = Store::open(root, options).unwrap();
    assert!(state.problem.is_some(), "the blocked config is a problem");
    store
}

/// Whether the store's config read allows writes and has the settings the
/// repair left (a config's own, else the defaults).
fn editable(store: &Store, case: &str) -> Result<(), String> {
    let config = store.config();
    if let Some(problem) = &config.problem {
        return Err(format!("{case}: still read-only ({:?})", problem.kind));
    }
    if config_write_failure(&config).is_some() {
        return Err(format!("{case}: writes refused"));
    }
    let org = config.config.preferred_format == Format::Org;
    if org != case.ends_with("with a config") {
        return Err(format!("{case}: config not read"));
    }
    Ok(())
}

#[test]
fn a_repaired_config_recovers_editing_on_refresh() {
    let mut wrong = Vec::new();
    for (case, steps) in REPAIRS {
        let temp = tempfile::tempdir().unwrap();
        blocked_disk_graph(temp.path());
        let store = open_disk(temp.path());
        repair_disk(temp.path(), steps);
        store.refresh(Depth::Stamps).unwrap();
        wrong.extend(editable(&store, case).err());
        store.close();
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_repaired_config_recovers_editing_on_the_next_reload() {
    let mut wrong = Vec::new();
    for (case, steps) in REPAIRS {
        let temp = tempfile::tempdir().unwrap();
        blocked_disk_graph(temp.path());
        let store = open_disk(temp.path());
        repair_disk(temp.path(), steps);
        // The poll watcher's next cycle; no refresh.
        let deadline = Instant::now() + Duration::from_secs(10);
        let answer = loop {
            match editable(&store, case) {
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                answer => break answer,
            }
        };
        wrong.extend(answer.err());
        store.close();
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_repaired_config_recovers_editing_in_memory() {
    for (case, steps) in REPAIRS {
        let mut files = Files::new();
        files.insert("pages/Note.md".into(), b"- first\n".to_vec());
        files.insert("logseq".into(), b"x".to_vec());
        let mut view = open(&files);
        assert!(view.config().problem.is_some(), "{case}");
        for step in steps {
            view.replace_files(apply(&mut files, std::slice::from_ref(step)))
                .unwrap();
        }
        assert!(view.config().problem.is_none(), "{case}");
        let org = view.config().config.preferred_format == Format::Org;
        assert_eq!(org, case.ends_with("with a config"), "{case}");
        let id = PageId::from("pages/Note.md");
        let mut dto = view.page(&id).unwrap();
        dto.blocks[0].raw = "edited".into();
        view.submit_page(&id, &dto, true).unwrap();
        assert_eq!(view.memory_page_bytes(&id), Some(b"- edited\n".to_vec()));
    }
}

/// Preservation: with no config.edn at all, the read's outcome stays
/// "absent", so a refresh reads and publishes no config change.
#[test]
fn an_absent_config_publishes_nothing_on_refresh() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("pages")).unwrap();
    std::fs::create_dir_all(temp.path().join("logseq")).unwrap();
    std::fs::write(temp.path().join("pages/a.md"), "- a\n").unwrap();
    let options = OpenOptions {
        approved_external_assets: None,
        watch: WatchMode::Poll,
        launch_checkpoint: None,
    };
    let (store, _, state) = Store::open(temp.path(), options).unwrap();
    assert!(state.problem.is_none());
    store.refresh(Depth::Stamps).unwrap();
    let subscription = store.subscribe();
    store.refresh(Depth::Stamps).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    while let Some(change) = subscription.try_recv().unwrap() {
        let config = (change.files.iter()).any(|(id, _, _)| id.as_str() == "logseq/config.edn");
        assert!(!config, "a config change was published: {change:?}");
    }
    store.close();
}

/// Preservation: a config.edn that is a link to a file is read through it at
/// open; removing the link keeps the config read from it, as removing a file
/// does (contract §2), rather than switching the graph to the defaults.
#[cfg(unix)]
#[test]
fn removing_a_linked_config_keeps_its_settings() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    std::fs::write(root.join("pages/a.md"), "- a\n").unwrap();
    std::fs::write(root.join("settings.edn"), "{:preferred-format \"Org\"}\n").unwrap();
    std::os::unix::fs::symlink("../settings.edn", root.join("logseq/config.edn")).unwrap();
    let options = OpenOptions {
        approved_external_assets: None,
        watch: WatchMode::Poll,
        launch_checkpoint: None,
    };
    let (store, _, state) = Store::open(root, options).unwrap();
    assert_eq!(state.config.preferred_format, Format::Org);
    std::fs::remove_file(root.join("logseq/config.edn")).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    let config = store.config();
    assert!(config.problem.is_none());
    assert_eq!(config.config.preferred_format, Format::Org);
    store.close();
}
