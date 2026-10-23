use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tine_store::{Area, OpenOptions, RestoreFile, Store, TrashKind};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "tine-store-restore-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn graph(root: &Path, external_assets: Option<&Path>) -> Store {
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    if external_assets.is_none() {
        fs::create_dir_all(root.join("assets")).unwrap();
    }
    Store::open(
        root,
        OpenOptions {
            approved_external_assets: external_assets.map(Path::to_path_buf),
            watch: Default::default(),
            launch_checkpoint: None,
        },
    )
    .unwrap()
    .0
}

fn input(path: &Path, area: Area, rel: &str) -> RestoreFile {
    RestoreFile {
        area,
        rel: rel.into(),
        bytes: std::fs::read(path).unwrap(),
    }
}

fn wait_paused(root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join(".tine-restore-test-paused").exists() {
        assert!(
            Instant::now() < deadline,
            "restore did not reach the bound-handle pause"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn recovery_dir(root: &Path) -> PathBuf {
    fs::read_dir(root.join("logseq/.tine-trash"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .unwrap()
}

/// F3's generated restore domain is deliberately separate from the page-host
/// state machine: journal -> page -> sidecar -> config, with unlisted extras.
pub(super) struct GeneratedRestore {
    pub(super) live: std::collections::BTreeMap<String, Vec<u8>>,
    pub(super) inputs: Vec<(Area, String, Vec<u8>)>,
}

impl GeneratedRestore {
    pub(super) fn assert_live_set(&self, root: &Path) {
        let expected: std::collections::BTreeMap<_, _> = self
            .inputs
            .iter()
            .map(|(area, rel, bytes)| (Self::path(*area, rel), bytes.clone()))
            .collect();
        let actual: std::collections::BTreeMap<_, _> = crate::conformance_f3::collect(root)
            .into_iter()
            .filter(|(rel, _)| {
                !rel.contains("/.")
                    && (rel.starts_with("pages/")
                        || rel.starts_with("journals/")
                        || rel.starts_with("assets/")
                        || rel == "logseq/config.edn")
            })
            .collect();
        assert_eq!(
            actual, expected,
            "successful restore must retire all extras"
        );
    }

    pub(super) fn backup(&self, root: &Path) {
        for (area, rel, bytes) in &self.inputs {
            let path = root.join(Self::path(*area, rel));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
    }

    pub(super) fn backup_intact(&self, root: &Path) {
        for (area, rel, bytes) in &self.inputs {
            assert_eq!(&fs::read(root.join(Self::path(*area, rel))).unwrap(), bytes);
        }
    }

    pub(super) fn files(&self) -> Vec<RestoreFile> {
        self.inputs
            .iter()
            .map(|(area, rel, bytes)| RestoreFile {
                area: *area,
                rel: rel.clone(),
                bytes: bytes.clone(),
            })
            .collect()
    }

    pub(super) fn path(area: Area, rel: &str) -> String {
        format!(
            "{}/{rel}",
            match area {
                Area::Pages => "pages",
                Area::Journals => "journals",
                Area::Assets => "assets",
                Area::Meta => "logseq",
                _ => unreachable!(),
            }
        )
    }
}

pub(super) fn generated_case(seed: usize) -> GeneratedRestore {
    use crate::conformance_f3::Rng;
    use std::collections::BTreeMap;
    let mut rng = Rng(0x7265_20261023 ^ seed as u64);
    let ext = if rng.below(2) == 0 { "md" } else { "org" };
    let bullet = if ext == "md" { "-" } else { "*" };
    let mut inputs = vec![
        (Area::Journals, format!("2026_10_23.{ext}")),
        (Area::Assets, "nested/paper.edn".to_owned()),
        (Area::Meta, "config.edn".to_owned()),
    ];
    let mut names: Vec<_> = (0..2 + rng.below(3))
        .map(|n| (Area::Pages, format!("Page{seed}-{n}.{ext}")))
        .collect();
    rng.shuffle(&mut names);
    inputs.extend(names);
    // The real protocol groups areas and preserves caller order inside each.
    inputs.sort_by_key(|(area, _)| match area {
        Area::Journals => 0,
        Area::Pages => 1,
        Area::Assets => 2,
        _ => 3,
    });
    let mut live = BTreeMap::new();
    let inputs = inputs
        .into_iter()
        .enumerate()
        .map(|(i, (area, rel))| {
            let old = match area {
                Area::Assets => format!("{{:payload \"old-{seed}-{i}\"}}\n"),
                Area::Meta => format!("{{:start-of-week {} :old {seed}}}\n", rng.below(7)),
                _ => format!("{bullet} old-{seed}-{i} [[Before]]\n"),
            };
            let new = match area {
                Area::Assets => format!("{{:payload \"new-{seed}-{i}\"}}\n"),
                Area::Meta => format!("{{:start-of-week {} :new {seed}}}\n", rng.below(7)),
                _ => format!("{bullet} new-{seed}-{i} [[After]]\n"),
            };
            live.insert(GeneratedRestore::path(area, &rel), old.into_bytes());
            (area, rel, new.into_bytes())
        })
        .collect();
    for (rel, bytes) in [
        (
            format!("pages/Extra.{ext}"),
            format!("{bullet} extra-page-{seed}\n"),
        ),
        (
            format!("journals/2026_10_22.{ext}"),
            format!("{bullet} extra-day-{seed}\n"),
        ),
        ("assets/extra.edn".into(), format!("{{:extra {seed}}}\n")),
    ] {
        live.insert(rel, bytes.into_bytes());
    }
    GeneratedRestore { live, inputs }
}

/// A process-local OS write limit makes copy_new fail during the selected
/// input's temp write, without a new hook in restore or ProductionIo. Earlier
/// inputs are below the limit; the selected input alone is padded past it.
#[cfg(unix)]
#[test]
fn generated_restore_copy_error_worker() {
    use crate::conformance_f3::{conserved, fresh};
    let _guard = crate::integration_lock();
    let Ok(seed) = std::env::var("TINE_F3_COPY_SEED") else {
        return;
    };
    let boundary: usize = std::env::var("TINE_F3_COPY_BOUNDARY")
        .unwrap()
        .parse()
        .unwrap();
    let root = PathBuf::from(std::env::var("TINE_CRASH_ROOT").unwrap());
    let mut case = generated_case(seed.parse().unwrap());
    case.inputs[boundary]
        .2
        .extend(std::iter::repeat_n(b' ', 8192));
    let store = Store::open(&root, Default::default()).unwrap().0;
    fresh(&store, &root, &["Before", "After"], "before copy error");
    // Only this disposable child changes its signal/resource state.
    unsafe {
        assert_ne!(libc::signal(libc::SIGXFSZ, libc::SIG_IGN), libc::SIG_ERR);
        assert_eq!(
            libc::setrlimit(
                libc::RLIMIT_FSIZE,
                &libc::rlimit {
                    rlim_cur: 4096,
                    rlim_max: 4096
                }
            ),
            0
        );
    }
    let error = store
        .restore(tine_store::EditKind::ReplacePage, case.files(), None)
        .unwrap_err();
    assert_eq!(
        error.cause.kind,
        std::io::ErrorKind::FileTooLarge,
        "{error:?}"
    );
    assert_eq!(error.done.restored as usize, boundary, "{error:?}");
    assert_eq!(error.done.recovery.len(), 2, "{error:?}");
    for bytes in case.live.values() {
        conserved(&root, bytes, "failed copy temp write");
    }
    // The failed input was detached before its temp write failed. Its original
    // bytes must be in a reported recovery root, and no partial temp is live.
    let (area, rel, _) = &case.inputs[boundary];
    assert!(!root.join(GeneratedRestore::path(*area, rel)).exists());
    let old = &case.live[&GeneratedRestore::path(*area, rel)];
    assert!(
        error
            .done
            .recovery
            .iter()
            .any(|dir| crate::conformance_f3::collect(dir)
                .values()
                .any(|bytes| bytes == old)),
        "{error:?}"
    );
    fresh(&store, &root, &["Before", "After"], "after copy error");
    store.close();
    eprintln!("F3 copy error reported at input {boundary}");
}

#[cfg(unix)]
#[test]
fn generated_restore_copy_errors_conserve_payloads_at_each_input() {
    use crate::conformance_f3::{conserved, fresh, seeds, Graph};
    let _guard = crate::integration_lock();
    let mut failures = 0;
    for seed in 1..=seeds() {
        let mut case = generated_case(seed);
        for boundary in 0..case.inputs.len() {
            let graph = Graph::new(&case.live);
            let backup = graph.dir.path().join("backup");
            let original = case.inputs[boundary].2.clone();
            case.inputs[boundary]
                .2
                .extend(std::iter::repeat_n(b' ', 8192));
            case.backup(&backup);
            graph.store.close();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "restore::generated_restore_copy_error_worker",
                    "--nocapture",
                ])
                .env("TINE_F3_COPY_SEED", seed.to_string())
                .env("TINE_F3_COPY_BOUNDARY", boundary.to_string())
                .env("TINE_CRASH_ROOT", &graph.root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "copy error seed {seed} boundary {boundary}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("F3 copy error reported at input")
            );
            case.backup_intact(&backup);
            for bytes in case.live.values() {
                conserved(&graph.root, bytes, "copy failure reopen");
            }
            let reopened = Store::open(&graph.root, Default::default()).unwrap().0;
            fresh(
                &reopened,
                &graph.root,
                &["Before", "After"],
                "copy failure reopen",
            );
            reopened.close();
            case.inputs[boundary].2 = original;
            failures += 1;
        }
    }
    eprintln!(
        "F3 restore copy errors: failures={failures}, image_checks={}",
        failures * 3
    );
}

#[cfg(feature = "test-faults")]
#[test]
fn generated_restore_step_errors_conserve_live_recovery_and_indexes() {
    use crate::conformance_f3::{collect, conserved, fresh, seeds, Graph};
    let _guard = crate::integration_lock();
    let mut failures = 0;
    let mut successes = 0;
    for seed in 1..=seeds() {
        let case = generated_case(seed);
        // One healthy control and an external recovery-name collision at each
        // input's retirement step, using restore's existing bound-handle pause.
        for boundary in 0..=case.inputs.len() {
            let graph = Graph::new(&case.live);
            let backup = graph.dir.path().join("backup");
            case.backup(&backup);
            let at = format!("restore seed {seed} boundary {boundary}");
            fresh(&graph.store, &graph.root, &["Before", "After"], &at);
            let fail = boundary < case.inputs.len();
            if fail {
                fs::write(graph.root.join(".tine-restore-test-pause"), b"pause").unwrap();
            }
            let result = std::thread::scope(|scope| {
                let task = scope.spawn(|| {
                    graph
                        .store
                        .restore(tine_store::EditKind::ReplacePage, case.files(), None)
                });
                if fail {
                    wait_paused(&graph.root);
                    let (area, rel, _) = &case.inputs[boundary];
                    let root = if *area == Area::Assets {
                        fs::read_dir(graph.root.join("assets/.tine-restore-recovery"))
                            .unwrap()
                            .next()
                            .unwrap()
                            .unwrap()
                            .path()
                    } else {
                        recovery_dir(&graph.root)
                    };
                    let rel = if *area == Area::Assets {
                        rel.clone()
                    } else {
                        GeneratedRestore::path(*area, rel)
                    };
                    let path = root.join(rel);
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(path, format!("external-recovery-{seed}-{boundary}")).unwrap();
                    fs::write(graph.root.join(".tine-restore-test-resume"), b"go").unwrap();
                }
                task.join().unwrap()
            });
            let report = if fail {
                let error = result.unwrap_err();
                assert_eq!(
                    error.cause.kind,
                    std::io::ErrorKind::AlreadyExists,
                    "{at}: {error:?}"
                );
                assert_eq!(error.done.restored as usize, boundary, "{at}: {error:?}");
                assert!(
                    error.done.kept_external.is_empty(),
                    "recovery collision is not a live copy collision: {at}"
                );
                conserved(
                    &graph.root,
                    format!("external-recovery-{seed}-{boundary}").as_bytes(),
                    &at,
                );
                failures += 1;
                error.done
            } else {
                let report = result.unwrap();
                assert_eq!(report.restored as usize, case.inputs.len());
                case.assert_live_set(&graph.root);
                successes += 1;
                report
            };
            assert_eq!(report.recovery.len(), 2, "{at}");
            case.backup_intact(&backup);
            // No old payload may hide in an unreported recovery directory.
            for (rel, bytes) in &case.live {
                let live = fs::read(graph.root.join(rel)).ok();
                assert!(
                    live.as_ref() == Some(bytes)
                        || report
                            .recovery
                            .iter()
                            .any(|root| collect(root).values().any(|b| b == bytes)),
                    "unreported/lost recovery {rel} at {at}"
                );
            }
            for (i, (area, rel, bytes)) in case.inputs.iter().enumerate() {
                if i < boundary {
                    assert_eq!(
                        &fs::read(graph.root.join(GeneratedRestore::path(*area, rel))).unwrap(),
                        bytes,
                        "{at}"
                    );
                }
            }
            fresh(&graph.store, &graph.root, &["Before", "After"], &at);
            graph.store.close();
            let reopened = Store::open(&graph.root, Default::default()).unwrap().0;
            fresh(&reopened, &graph.root, &["Before", "After"], &at);
            reopened.close();
        }
    }
    assert_eq!(successes, seeds());
    eprintln!(
        "F3 restore: failures={failures}, successes={successes}, image_checks={}",
        (failures + successes) * 3
    );
}

#[test]
fn restore_recovery_roots_live_on_the_filesystems_they_detach_from() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("recovery-roots");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    fs::write(graph_root.join("pages/secret.md"), b"live").unwrap();
    fs::write(graph_root.join("assets/doc.edn"), b"live").unwrap();
    let report = store
        .restore(tine_store::EditKind::ReplacePage, Vec::new(), None)
        .unwrap();
    assert_eq!(report.recovery.len(), 2);
    // Recovery roots are reported under the canonical root `Store::open`
    // binds (on Windows a `\\?\` long-name path).
    let canonical = fs::canonicalize(&graph_root).unwrap();
    assert!(report.recovery[0].starts_with(canonical.join("logseq").join(".tine-trash")));
    assert!(report.recovery[1].starts_with(canonical.join("assets").join(".tine-restore-recovery")));
    assert_eq!(
        fs::read(report.recovery[0].join("pages/secret.md")).unwrap(),
        b"live"
    );
    assert_eq!(
        fs::read(report.recovery[1].join("doc.edn")).unwrap(),
        b"live"
    );
    let stats = store.trash_stats().unwrap();
    assert!(stats
        .iter()
        .any(|(kind, count, bytes)| { *kind == TrashKind::Legacy && *count >= 1 && *bytes >= 4 }));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_does_not_publish_recovery_sidecars_as_live_assets() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("recovery-feed");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    fs::write(graph_root.join("assets/doc.edn"), b"live").unwrap();
    store.whole_graph().unwrap();
    let subscription = store.subscribe();
    store
        .restore(tine_store::EditKind::ReplacePage, Vec::new(), None)
        .unwrap();
    let change = subscription
        .try_recv()
        .unwrap()
        .expect("restore publication");
    assert!(!change
        .files
        .iter()
        .any(|(id, _, _)| { id.as_str().starts_with("assets/.tine-restore-recovery/") }));
    store.close();
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_refuses_config_with_unsafe_directories_before_retiring_pages() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("unsafe-config");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    fs::write(graph_root.join("pages/live.md"), b"- keep me\n").unwrap();
    let source = root.join("bad-config.edn");
    fs::write(&source, b"{:pages-directory \"../outside\"}\n").unwrap();
    assert!(store
        .restore(
            tine_store::EditKind::ReplacePage,
            vec![input(&source, Area::Meta, "config.edn")],
            None
        )
        .is_err());
    assert_eq!(
        fs::read(graph_root.join("pages/live.md")).unwrap(),
        b"- keep me\n"
    );
    assert!(!graph_root.join("logseq/config.edn").exists());
    store.close();
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_journal_updates_day_and_view() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("journal-index");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    store.whole_graph().unwrap();
    let source = root.join("journal.org");
    fs::write(&source, b"* restored\n").unwrap();
    store
        .restore(
            tine_store::EditKind::ReplacePage,
            vec![input(&source, Area::Journals, "2026_09_25.org")],
            None,
        )
        .unwrap();
    assert_eq!(
        store.journal_id(tine_store::Day(20260925)).as_str(),
        "journals/2026_09_25.org"
    );
    let view = store.whole_graph().unwrap();
    assert!(
        matches!(view.resolve("Sep 25th, 2026", true), tine_store::Resolved::Existing { id, .. } if id.as_str() == "journals/2026_09_25.org")
    );
    assert!(view
        .inventory(tine_store::InventoryScope::All)
        .0
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
    assert!(view
        .complete_page_names("Sep 25", 10, None, false, None)
        .into_iter()
        .map(|(page, _)| page)
        .collect::<Vec<_>>()
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
    store.close();
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn restore_recovery_symlink_cannot_redirect_or_replace_outside() {
    let _integration_guard = crate::integration_lock();
    use std::os::unix::fs::symlink;
    let root = scratch("recovery-symlink");
    let graph_root = root.join("graph");
    let outside = root.join("outside");
    let store = graph(&graph_root, None);
    let live = graph_root.join("pages/secret.md");
    fs::write(&live, b"live graph data").unwrap();
    fs::create_dir_all(outside.join("restore-1/pages")).unwrap();
    let outside_target = outside.join("restore-1/pages/secret.md");
    fs::write(&outside_target, b"outside sentinel").unwrap();
    symlink(&outside, graph_root.join("logseq/.tine-trash")).unwrap();
    assert!(store
        .restore(tine_store::EditKind::ReplacePage, Vec::new(), None)
        .is_err());
    assert_eq!(fs::read(&live).unwrap(), b"live graph data");
    assert_eq!(fs::read(&outside_target).unwrap(), b"outside sentinel");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn restore_recovery_path_swap_stays_on_the_bound_directory() {
    let _integration_guard = crate::integration_lock();
    use std::os::unix::fs::symlink;
    let root = scratch("recovery-swap");
    let graph_root = root.join("graph");
    let outside = root.join("outside");
    let store = graph(&graph_root, None);
    fs::write(graph_root.join("pages/secret.md"), b"live graph data").unwrap();
    fs::create_dir_all(outside.join("pages")).unwrap();
    fs::write(outside.join("pages/secret.md"), b"outside sentinel").unwrap();
    fs::write(graph_root.join(".tine-restore-test-pause"), b"pause").unwrap();
    std::thread::scope(|scope| {
        let task =
            scope.spawn(|| store.restore(tine_store::EditKind::ReplacePage, Vec::new(), None));
        wait_paused(&graph_root);
        let recovery = recovery_dir(&graph_root);
        let displaced = recovery.with_extension("displaced");
        fs::rename(&recovery, &displaced).unwrap();
        symlink(&outside, &recovery).unwrap();
        fs::write(graph_root.join(".tine-restore-test-resume"), b"resume").unwrap();
        task.join().unwrap().unwrap();
        assert!(!graph_root.join("pages/secret.md").exists());
        assert_eq!(
            fs::read(displaced.join("pages/secret.md")).unwrap(),
            b"live graph data"
        );
        assert_eq!(
            fs::read(outside.join("pages/secret.md")).unwrap(),
            b"outside sentinel"
        );
    });
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn restore_live_path_swap_cannot_move_or_publish_outside() {
    let _integration_guard = crate::integration_lock();
    use std::os::unix::fs::symlink;
    let root = scratch("live-swap");
    let graph_root = root.join("graph");
    let outside = root.join("outside");
    let store = graph(&graph_root, None);
    let pages = graph_root.join("pages");
    let snapshot = root.join("snapshot.md");
    fs::create_dir_all(&outside).unwrap();
    fs::write(pages.join("secret.md"), b"live graph data").unwrap();
    fs::write(outside.join("secret.md"), b"outside sentinel").unwrap();
    fs::write(&snapshot, b"snapshot data").unwrap();
    fs::write(graph_root.join(".tine-restore-test-pause"), b"pause").unwrap();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| {
            store.restore(
                tine_store::EditKind::ReplacePage,
                vec![input(&snapshot, Area::Pages, "new.md")],
                None,
            )
        });
        wait_paused(&graph_root);
        let displaced = graph_root.join("pages.displaced");
        fs::rename(&pages, &displaced).unwrap();
        symlink(&outside, &pages).unwrap();
        fs::write(graph_root.join(".tine-restore-test-resume"), b"resume").unwrap();
        assert!(task.join().unwrap().is_err());
        assert_eq!(
            fs::read(displaced.join("secret.md")).unwrap(),
            b"live graph data"
        );
        assert_eq!(
            fs::read(outside.join("secret.md")).unwrap(),
            b"outside sentinel"
        );
        assert!(!outside.join("new.md").exists());
    });
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_recovery_never_replaces_an_existing_entry() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("recovery-no-replace");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    let live = graph_root.join("pages/secret.md");
    let snapshot = root.join("snapshot.md");
    fs::write(&live, b"live graph data").unwrap();
    fs::write(&snapshot, b"snapshot data").unwrap();
    fs::write(graph_root.join(".tine-restore-test-pause"), b"pause").unwrap();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| {
            store.restore(
                tine_store::EditKind::ReplacePage,
                vec![input(&snapshot, Area::Pages, "secret.md")],
                None,
            )
        });
        wait_paused(&graph_root);
        let recovery = recovery_dir(&graph_root);
        fs::create_dir_all(recovery.join("pages")).unwrap();
        fs::write(recovery.join("pages/secret.md"), b"recovery sentinel").unwrap();
        fs::write(graph_root.join(".tine-restore-test-resume"), b"resume").unwrap();
        assert!(task.join().unwrap().is_err());
        assert_eq!(fs::read(&live).unwrap(), b"live graph data");
        assert_eq!(
            fs::read(recovery.join("pages/secret.md")).unwrap(),
            b"recovery sentinel"
        );
    });
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_restore_reports_completed_files_and_preserves_the_failing_target() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("partial-report");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    let first = root.join("first.md");
    let second = root.join("second.md");
    fs::write(&first, b"new first").unwrap();
    fs::write(&second, b"new second").unwrap();
    fs::write(graph_root.join("pages/First.md"), b"old first").unwrap();
    fs::write(graph_root.join("pages/Second.md"), b"old second").unwrap();
    fs::write(graph_root.join(".tine-restore-test-pause"), b"pause").unwrap();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| {
            store.restore(
                tine_store::EditKind::ReplacePage,
                vec![
                    input(&first, Area::Pages, "First.md"),
                    input(&second, Area::Pages, "Second.md"),
                ],
                None,
            )
        });
        wait_paused(&graph_root);
        let recovery = recovery_dir(&graph_root);
        fs::create_dir_all(recovery.join("pages")).unwrap();
        fs::write(recovery.join("pages/Second.md"), b"recovery sentinel").unwrap();
        fs::write(graph_root.join(".tine-restore-test-resume"), b"resume").unwrap();
        let failed = task.join().unwrap().err().expect("second file must fail");
        assert_eq!(failed.phase, "restore pages failed");
        assert_eq!(failed.done.restored, 1);
        assert!(failed.done.kept_external.is_empty());
        assert_eq!(
            fs::read(graph_root.join("pages/First.md")).unwrap(),
            b"new first"
        );
        assert_eq!(
            fs::read(graph_root.join("pages/Second.md")).unwrap(),
            b"old second"
        );
        assert_eq!(
            fs::read(recovery.join("pages/First.md")).unwrap(),
            b"old first"
        );
    });
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_asset_sidecars_dir_restores_sidecars_and_leaves_binary_assets() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("sidecars");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    let assets = graph_root.join("assets");
    let snapshot = root.join("snapshot");
    fs::create_dir_all(assets.join("nested")).unwrap();
    fs::create_dir_all(snapshot.join("nested")).unwrap();
    fs::write(snapshot.join("doc.edn"), b"new\n").unwrap();
    fs::write(snapshot.join("nested/hl.edn"), b"nested new\n").unwrap();
    fs::write(assets.join("doc.edn"), b"old\n").unwrap();
    fs::write(assets.join("stale.edn"), b"stale\n").unwrap();
    fs::write(assets.join("nested/stale.edn"), b"stale\n").unwrap();
    fs::write(assets.join("image.png"), b"keep").unwrap();
    fs::write(assets.join("nested/image.png"), b"keep").unwrap();
    let report = store
        .restore(
            tine_store::EditKind::ReplacePage,
            vec![
                input(&snapshot.join("doc.edn"), Area::Assets, "doc.edn"),
                input(
                    &snapshot.join("nested/hl.edn"),
                    Area::Assets,
                    "nested/hl.edn",
                ),
            ],
            None,
        )
        .unwrap();
    assert_eq!(report.restored, 2);
    assert_eq!(fs::read(assets.join("doc.edn")).unwrap(), b"new\n");
    assert_eq!(
        fs::read(assets.join("nested/hl.edn")).unwrap(),
        b"nested new\n"
    );
    assert!(!assets.join("stale.edn").exists());
    assert!(!assets.join("nested/stale.edn").exists());
    assert_eq!(
        fs::read(report.recovery[1].join("stale.edn")).unwrap(),
        b"stale\n"
    );
    assert_eq!(fs::read(assets.join("image.png")).unwrap(), b"keep");
    assert_eq!(fs::read(assets.join("nested/image.png")).unwrap(), b"keep");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn graph_text_backup_and_restore_include_nested_pages() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("nested-pages");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    let snapshot = root.join("snapshot");
    fs::create_dir_all(snapshot.join("client-a")).unwrap();
    fs::create_dir_all(graph_root.join("pages/client-a")).unwrap();
    fs::write(snapshot.join("Top.md"), b"top\n").unwrap();
    fs::write(snapshot.join("client-a/Deep.md"), b"deep\n").unwrap();
    fs::write(graph_root.join("pages/client-a/Deep.md"), b"corrupt\n").unwrap();
    fs::write(graph_root.join("pages/client-a/Stale.md"), b"stale\n").unwrap();
    fs::write(graph_root.join("pages/client-a/notes.txt"), b"keep\n").unwrap();
    let report = store
        .restore(
            tine_store::EditKind::ReplacePage,
            vec![
                input(&snapshot.join("Top.md"), Area::Pages, "Top.md"),
                input(
                    &snapshot.join("client-a/Deep.md"),
                    Area::Pages,
                    "client-a/Deep.md",
                ),
            ],
            None,
        )
        .unwrap();
    assert_eq!(
        fs::read(graph_root.join("pages/client-a/Deep.md")).unwrap(),
        b"deep\n"
    );
    assert!(!graph_root.join("pages/client-a/Stale.md").exists());
    assert_eq!(
        fs::read(report.recovery[0].join("pages/client-a/Stale.md")).unwrap(),
        b"stale\n"
    );
    assert_eq!(
        fs::read(graph_root.join("pages/client-a/notes.txt")).unwrap(),
        b"keep\n"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn complete_restore_crosses_from_app_data_to_a_distinct_live_filesystem() {
    let _integration_guard = crate::integration_lock();
    use std::os::unix::fs::MetadataExt;
    let app_data = scratch("cross-device-source");
    let live_root = PathBuf::from("/dev/shm").join(format!(
        "tine-restore-cross-device-live-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&live_root);
    if fs::create_dir_all(&live_root).is_err()
        || fs::metadata(&app_data).unwrap().dev() == fs::metadata(&live_root).unwrap().dev()
    {
        let _ = fs::remove_dir_all(&app_data);
        let _ = fs::remove_dir_all(&live_root);
        return;
    }
    let snapshot = app_data.join("snapshot");
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(snapshot.join(dir)).unwrap();
    }
    let store = graph(&live_root, None);
    for (rel, bytes) in [
        ("pages/Kept.md", b"snapshot page\n".as_slice()),
        ("journals/2026_07_15.md", b"snapshot journal\n"),
        ("assets/doc.edn", b"{:snapshot true}\n"),
        ("logseq/config.edn", b"{:snapshot true}\n"),
    ] {
        fs::write(snapshot.join(rel), bytes).unwrap();
    }
    for (rel, bytes) in [
        ("pages/Kept.md", b"live page\n".as_slice()),
        ("pages/Stale.md", b"stale page\n"),
        ("journals/Old.md", b"old journal\n"),
        ("assets/doc.edn", b"{:live true}\n"),
        ("assets/stale.edn", b"{:stale true}\n"),
        ("assets/binary.pdf", b"keep binary"),
        ("logseq/config.edn", b"{:live true}\n"),
    ] {
        fs::write(live_root.join(rel), bytes).unwrap();
    }
    let report = store
        .restore(
            tine_store::EditKind::ReplacePage,
            vec![
                input(
                    &snapshot.join("journals/2026_07_15.md"),
                    Area::Journals,
                    "2026_07_15.md",
                ),
                input(&snapshot.join("pages/Kept.md"), Area::Pages, "Kept.md"),
                input(&snapshot.join("assets/doc.edn"), Area::Assets, "doc.edn"),
                input(
                    &snapshot.join("logseq/config.edn"),
                    Area::Meta,
                    "config.edn",
                ),
            ],
            None,
        )
        .unwrap();
    assert_eq!(
        fs::read(live_root.join("pages/Kept.md")).unwrap(),
        b"snapshot page\n"
    );
    assert!(!live_root.join("pages/Stale.md").exists());
    assert_eq!(
        fs::read(live_root.join("journals/2026_07_15.md")).unwrap(),
        b"snapshot journal\n"
    );
    assert!(!live_root.join("journals/Old.md").exists());
    assert_eq!(
        fs::read(live_root.join("assets/doc.edn")).unwrap(),
        b"{:snapshot true}\n"
    );
    assert!(!live_root.join("assets/stale.edn").exists());
    assert_eq!(
        fs::read(live_root.join("assets/binary.pdf")).unwrap(),
        b"keep binary"
    );
    assert_eq!(
        fs::read(live_root.join("logseq/config.edn")).unwrap(),
        b"{:snapshot true}\n"
    );
    assert_eq!(
        fs::read(report.recovery[0].join("pages/Stale.md")).unwrap(),
        b"stale page\n"
    );
    assert_eq!(
        fs::read(report.recovery[0].join("logseq/config.edn")).unwrap(),
        b"{:live true}\n"
    );
    assert_eq!(
        fs::read(report.recovery[1].join("stale.edn")).unwrap(),
        b"{:stale true}\n"
    );
    drop(store);
    fs::remove_dir_all(app_data).unwrap();
    fs::remove_dir_all(live_root).unwrap();
}

#[cfg(unix)]
#[test]
fn approved_external_assets_restore_keeps_recovery_on_target() {
    let _integration_guard = crate::integration_lock();
    use std::os::unix::fs::symlink;
    let root = scratch("external-assets");
    let graph_root = root.join("graph");
    let assets = root.join("external-assets");
    fs::create_dir_all(&graph_root).unwrap();
    fs::create_dir_all(&assets).unwrap();
    symlink(&assets, graph_root.join("assets")).unwrap();
    let store = graph(&graph_root, Some(&assets));
    let source = root.join("source.edn");
    fs::write(&source, b"new").unwrap();
    fs::write(assets.join("old.edn"), b"old").unwrap();
    let report = store
        .restore(
            tine_store::EditKind::ReplacePage,
            vec![input(&source, Area::Assets, "new.edn")],
            None,
        )
        .unwrap();
    assert_eq!(fs::read(assets.join("new.edn")).unwrap(), b"new");
    assert!(!assets.join("old.edn").exists());
    assert!(report.recovery[1].starts_with(assets.join(".tine-restore-recovery")));
    assert_eq!(
        fs::read(report.recovery[1].join("old.edn")).unwrap(),
        b"old"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// og-B whole-graph restore (ADR 0062): text returns to its graph-relative
/// path outside `pages/` and `journals/`, and the unlisted live text retired
/// is decided by the scope recorded at snapshot time, not by today's
/// `:hidden`. A page hidden when the snapshot was taken was never captured,
/// so it must stay live.
#[test]
fn graph_restore_places_text_anywhere_and_retires_by_the_recorded_scope() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("graph-wide");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    fs::create_dir_all(graph_root.join("archive/deep")).unwrap();
    fs::create_dir_all(graph_root.join("private")).unwrap();
    fs::write(graph_root.join("Root.md"), "old root").unwrap();
    fs::write(graph_root.join("archive/Stale.md"), "stale").unwrap();
    fs::write(graph_root.join("private/Secret.md"), "secret").unwrap();
    fs::write(graph_root.join("pages/Kept.md"), "kept old").unwrap();
    let snapshot = root.join("snapshot");
    fs::create_dir_all(&snapshot).unwrap();
    fs::write(snapshot.join("Root.md"), "new root").unwrap();
    fs::write(snapshot.join("Deep.org"), "* deep").unwrap();
    fs::write(snapshot.join("Kept.md"), "kept new").unwrap();
    let recorded = ["private".to_owned()];
    let report = store
        .restore(
            tine_store::EditKind::ReplacePage,
            vec![
                input(&snapshot.join("Root.md"), Area::Graph, "Root.md"),
                input(
                    &snapshot.join("Deep.org"),
                    Area::Graph,
                    "archive/deep/Deep.org",
                ),
                input(&snapshot.join("Kept.md"), Area::Graph, "pages/Kept.md"),
            ],
            Some(&recorded),
        )
        .unwrap();
    assert_eq!(report.restored, 3);
    assert_eq!(fs::read(graph_root.join("Root.md")).unwrap(), b"new root");
    assert_eq!(
        fs::read(graph_root.join("archive/deep/Deep.org")).unwrap(),
        b"* deep"
    );
    assert_eq!(
        fs::read(graph_root.join("pages/Kept.md")).unwrap(),
        b"kept new"
    );
    assert!(!graph_root.join("archive/Stale.md").exists());
    let recovery = recovery_dir(&graph_root);
    assert_eq!(
        fs::read(recovery.join("graph/archive/Stale.md")).unwrap(),
        b"stale"
    );
    assert_eq!(
        fs::read(recovery.join("graph/Root.md")).unwrap(),
        b"old root"
    );
    assert_eq!(
        fs::read(graph_root.join("private/Secret.md")).unwrap(),
        b"secret",
        "I-2: text outside the recorded scope was never captured and must stay live"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// Refusal (malformed snapshot content — a torn or mixed snapshot): a
/// whole-graph restore takes graph text only as `Area::Graph` inside the
/// recorded scope, and a configured-roots restore never takes `Area::Graph`.
/// Either mismatch refuses before any live file moves.
#[test]
fn graph_restore_refuses_text_outside_its_recorded_scope() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("graph-refuse");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    fs::write(graph_root.join("pages/Live.md"), "live").unwrap();
    let source = root.join("New.md");
    fs::write(&source, "new").unwrap();
    let recorded = ["private".to_owned()];
    for (area, rel, scope) in [
        (Area::Pages, "New.md", Some(&recorded[..])),
        (Area::Graph, "private/New.md", Some(&recorded[..])),
        (Area::Graph, "assets/New.md", Some(&[][..])),
        (Area::Graph, "logseq/.tine-trash/New.md", Some(&[][..])),
        (Area::Graph, "New.md", None),
    ] {
        assert!(store
            .restore(
                tine_store::EditKind::ReplacePage,
                vec![input(&source, area, rel)],
                scope,
            )
            .is_err());
        assert_eq!(fs::read(graph_root.join("pages/Live.md")).unwrap(), b"live");
    }
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// REG-OG-C5-L06-S1 (I-1, I-2): restore retires the live file into a recovery
/// tree created for this restore, then publishes the backup bytes. Scenario:
/// power loss after the replacement is durable — unless the recovery entry and
/// every directory created for it were synced first, the retired original (an
/// external editor's edit newer than the safety snapshot) can vanish. A
/// process kill cannot show this; the recorded directory syncs can.
#[test]
fn restore_makes_the_recovery_tree_durable_before_publishing_replacements() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("recovery-durable");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    fs::create_dir_all(graph_root.join("pages/sub")).unwrap();
    fs::write(graph_root.join("pages/sub/A.md"), b"- external edit\n").unwrap();
    fs::write(graph_root.join("pages/B.md"), b"- unlisted\n").unwrap();
    let source = root.join("A.md");
    fs::write(&source, b"- from backup\n").unwrap();
    tine_store::directory_durability::take_synced_directories();
    let report = store
        .restore(
            tine_store::EditKind::ReplacePage,
            vec![input(&source, Area::Pages, "sub/A.md")],
            None,
        )
        .unwrap();
    let synced = tine_store::directory_durability::take_synced_directories();
    let recovery = &report.recovery[0];
    let canonical = fs::canonicalize(&graph_root).unwrap();
    assert_eq!(
        fs::read(recovery.join("pages/sub/A.md")).unwrap(),
        b"- external edit\n"
    );
    assert_eq!(
        fs::read(recovery.join("pages/B.md")).unwrap(),
        b"- unlisted\n"
    );
    let first = |dir: &Path| synced.iter().position(|path| path == dir);
    let live_publish =
        first(&canonical.join("pages/sub")).expect("the replacement's live directory is synced");
    for dir in [
        recovery.join("pages/sub"),
        recovery.join("pages"),
        recovery.clone(),
        canonical.join("logseq/.tine-trash"),
        canonical.join("logseq"),
    ] {
        let at = first(&dir).unwrap_or_else(|| {
            panic!(
                "I-2: restore recovery entry {} never synced; exemplar restore.rs reserve/move_if_present; synced {synced:?}",
                dir.display()
            )
        });
        assert!(
            at < live_publish,
            "I-2: recovery entry {} synced only after the replacement was published",
            dir.display()
        );
    }
    // The unlisted page is retired after the copy; its recovery entry is synced too.
    let last_recovery_pages = synced
        .iter()
        .rposition(|path| *path == recovery.join("pages"))
        .unwrap();
    assert!(
        last_recovery_pages > live_publish,
        "unlisted retirement synced: {synced:?}"
    );
    store.close();
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
