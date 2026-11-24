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

// The one state rule (contract §2): settings change only on a successful
// read; absent and unreadable keep the settings in force; the problem is the
// current outcome's alone. Each sequence starts from a graph whose config
// lists the favorite `A`.

/// One observation step: the files it writes (`Some`) or removes (`None`),
/// in order, and what the graph serves after it: the favorite its settings
/// list (`None` for the defaults) and whether a read problem holds it
/// read-only. A directory exists while it holds a file, as in memory.
type Step = (
    &'static [(&'static str, Option<&'static str>)],
    Option<&'static str>,
    bool,
);

const CONFIG_A: &str = "{:favorites [\"A\"]}\n";

const KEPT_SEQUENCES: [(&str, &[Step]); 2] = [
    (
        "a directory takes config.edn's place, then goes",
        &[
            (&[("logseq/config.edn", None)], Some("a"), false),
            (&[("logseq/config.edn/x", Some("x"))], Some("a"), true),
            (&[("logseq/config.edn/x", None)], Some("a"), false),
        ],
    ),
    (
        "a file takes logseq's place, then goes",
        &[
            (
                &[("logseq/config.edn", None), ("logseq", Some("x"))],
                Some("a"),
                true,
            ),
            (&[("logseq", None)], Some("a"), false),
            (
                &[("logseq/config.edn", Some("{:favorites [\"B\"]}\n"))],
                Some("b"),
                false,
            ),
        ],
    ),
];

/// `ops` on disk under `root`; removing a directory's last file removes the
/// directory, as a memory tree has no empty directory.
fn apply_disk(root: &Path, ops: &[(&str, Option<&str>)]) {
    for (rel, text) in ops {
        let path = root.join(rel);
        match text {
            Some(text) => {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            None => {
                std::fs::remove_file(&path).unwrap();
                let mut dir = path.parent().unwrap();
                while dir != root && std::fs::remove_dir(dir).is_ok() {
                    dir = dir.parent().unwrap();
                }
            }
        }
    }
}

fn served(config: &super::ConfigState) -> (Option<String>, bool) {
    let favorite = config
        .config
        .favorites
        .first()
        .map(|name| name.to_lowercase());
    (favorite, config.problem.is_some())
}

fn expected(step: &Step) -> (Option<String>, bool) {
    (step.1.map(str::to_owned), step.2)
}

fn open_poll(root: &Path) -> Store {
    let options = OpenOptions {
        approved_external_assets: None,
        watch: WatchMode::Poll,
        launch_checkpoint: None,
    };
    Store::open(root, options).unwrap().0
}

fn kept_disk_graph(root: &Path) {
    apply_disk(
        root,
        &[
            ("pages/a.md", Some("- a\n")),
            ("logseq/config.edn", Some(CONFIG_A)),
        ],
    );
}

/// Waits for the poll watcher's next cycles to serve `want`; no refresh.
fn polled(store: &Store, want: &(Option<String>, bool)) -> (Option<String>, bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let now = served(&store.config());
        if &now == want || Instant::now() >= deadline {
            return now;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_blocked_read_keeps_the_settings_on_refresh() {
    let mut wrong = Vec::new();
    for (case, steps) in KEPT_SEQUENCES {
        let temp = tempfile::tempdir().unwrap();
        kept_disk_graph(temp.path());
        let store = open_poll(temp.path());
        for (at, step) in steps.iter().enumerate() {
            apply_disk(temp.path(), step.0);
            store.refresh(Depth::Stamps).unwrap();
            let now = served(&store.config());
            if now != expected(step) {
                wrong.push(format!(
                    "{case}, step {at}: {now:?}, want {:?}",
                    expected(step)
                ));
            }
        }
        store.close();
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_blocked_read_keeps_the_settings_on_the_next_poll() {
    let mut wrong = Vec::new();
    for (case, steps) in KEPT_SEQUENCES {
        let temp = tempfile::tempdir().unwrap();
        kept_disk_graph(temp.path());
        let store = open_poll(temp.path());
        for (at, step) in steps.iter().enumerate() {
            apply_disk(temp.path(), step.0);
            let now = polled(&store, &expected(step));
            if now != expected(step) {
                wrong.push(format!(
                    "{case}, step {at}: {now:?}, want {:?}",
                    expected(step)
                ));
            }
        }
        store.close();
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_blocked_read_keeps_the_settings_in_memory() {
    let mut wrong = Vec::new();
    for (case, steps) in KEPT_SEQUENCES {
        let mut files = Files::new();
        files.insert("pages/a.md".into(), b"- a\n".to_vec());
        files.insert("logseq/config.edn".into(), CONFIG_A.as_bytes().to_vec());
        let mut view = open(&files);
        for (at, step) in steps.iter().enumerate() {
            for op in step.0 {
                let changes = apply(&mut files, std::slice::from_ref(op));
                view.replace_files(changes).unwrap();
            }
            let now = served(view.config());
            if now != expected(step) {
                wrong.push(format!(
                    "{case}, step {at}: {now:?}, want {:?}",
                    expected(step)
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A graph opened blocked (a `logseq` file) is repaired by a `logseq`
/// directory whose config.edn is a dangling link, and then the link's target
/// is delivered: the read follows the link, so the dangling link is absent
/// (editable, the defaults) and the delivered target is read.
#[cfg(unix)]
fn dangling_link_repair(
    store: &Store,
    root: &Path,
    wait: impl Fn(&Store, &(Option<String>, bool)) -> (Option<String>, bool),
) -> Vec<String> {
    let mut wrong = Vec::new();
    std::fs::remove_file(root.join("logseq")).unwrap();
    std::fs::create_dir(root.join("logseq")).unwrap();
    std::os::unix::fs::symlink("../settings.edn", root.join("logseq/config.edn")).unwrap();
    let want = (None, false);
    let now = wait(store, &want);
    if now != want {
        wrong.push(format!("dangling link: {now:?}, want {want:?}"));
    }
    std::fs::write(root.join("settings.edn"), CONFIG_A).unwrap();
    let want = (Some("a".to_owned()), false);
    let now = wait(store, &want);
    if now != want {
        wrong.push(format!("target delivered: {now:?}, want {want:?}"));
    }
    wrong
}

#[cfg(unix)]
#[test]
fn a_dangling_config_link_repairs_on_refresh_and_on_the_next_poll() {
    let refresh = |store: &Store, _: &(Option<String>, bool)| {
        store.refresh(Depth::Stamps).unwrap();
        served(&store.config())
    };
    let mut wrong = Vec::new();
    for (how, poll) in [("refresh", false), ("next poll", true)] {
        let temp = tempfile::tempdir().unwrap();
        blocked_disk_graph(temp.path());
        let store = open_disk(temp.path());
        let found = if poll {
            dangling_link_repair(&store, temp.path(), polled)
        } else {
            dangling_link_repair(&store, temp.path(), refresh)
        };
        wrong.extend(found.into_iter().map(|line| format!("{how}: {line}")));
        store.close();
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A config that became unreadable (no permission) holds the graph
/// read-only on the settings in force; removing it clears the problem. The
/// stamp observed the unreadable file as what the read found, not as absent.
#[cfg(unix)]
#[test]
fn a_removed_unreadable_config_clears_its_problem() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    kept_disk_graph(root);
    let store = open_poll(root);
    let config = root.join("logseq/config.edn");
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&config).is_ok() {
        store.close();
        return; // Running with privileges that read it anyway.
    }
    let kept = |problem| (Some("a".to_owned()), problem);
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(served(&store.config()), kept(true), "unreadable");
    std::fs::remove_file(&config).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(served(&store.config()), kept(false), "removed");
    store.close();
}
