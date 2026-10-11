use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tine-tauri-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &std::path::Path, bytes: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A small graph with a store, and its backup base inside a scratch app
/// data dir (`<root>/app/backups/<id>`).
struct Graph {
    root: PathBuf,
    graph: PathBuf,
    base: PathBuf,
    store: Store,
    source: BackupSource,
}

impl Graph {
    fn new(tag: &str) -> Self {
        let root = scratch(tag);
        let graph = root.join("graph");
        for dir in ["pages", "journals", "assets", "logseq"] {
            std::fs::create_dir_all(graph.join(dir)).unwrap();
        }
        write(&graph.join("pages/A.md"), "- a\n");
        let store = Store::open(&graph, Default::default()).unwrap().0;
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let base = root.join("app/backups").join(root_backup_id(&source.root));
        Self {
            root,
            graph,
            base,
            store,
            source,
        }
    }

    fn snap(&self, suffix: &str, keep: usize) -> BackupOutcome {
        self.snap_waiting(suffix, keep, std::time::Duration::from_secs(30))
    }

    fn snap_waiting(&self, suffix: &str, keep: usize, wait: std::time::Duration) -> BackupOutcome {
        write_snapshot(
            &self.base,
            &self.store,
            self.source.clone(),
            suffix,
            &|| false,
            keep,
            wait,
        )
    }

    fn zips(&self) -> PathBuf {
        zip_base(&self.base)
    }

    /// Every name in `<id>.zips/` but the lock, sorted.
    fn zip_names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.zips())
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != naming::LOCK_FILE)
            .collect();
        names.sort();
        names
    }

    /// Zip and legacy entry names, oldest first.
    fn all_names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.base)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.extend(self.zip_names());
        names.sort();
        names
    }

    fn newest_zip(&self) -> PathBuf {
        newest_zip_in(&self.base, &self.graph)
    }

    fn close(self) {
        self.store.close();
        drop(self.store);
        let _ = std::fs::remove_dir_all(self.root);
    }
}

/// The newest zip in listing order (stamp, then numeric counter): a path's
/// lexical order puts `<stamp>.zip` after `<stamp>-2.zip`.
fn newest_zip_in(base: &std::path::Path, graph: &std::path::Path) -> PathBuf {
    let listed = list_backups_from_base(base, graph);
    let id = &listed
        .iter()
        .find(|info| info.id.starts_with("zip:"))
        .unwrap()
        .id;
    zip_base(base).join(format!("{}.zip", &id["zip:".len()..]))
}

fn verified(path: &std::path::Path) -> archive::Verified {
    archive::verify(&mut &read_archive(path).unwrap()[..]).unwrap()
}

fn ops() -> Vec<&'static str> {
    BACKUP_OPS.with(|ops| std::mem::take(&mut *ops.borrow_mut()))
}

fn hook(op: &'static str, hook: Hook) {
    HOOKS.with(|hooks| hooks.borrow_mut().push((op, hook)));
}

fn legacy_dir(base: &std::path::Path, name: &str, schema: u32, writer: &str) {
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(SNAPSHOT_MANIFEST),
        format!(r#"{{"schema":{schema},"root":"/g","journals_dir":"journals","pages_dir":"pages",{writer}"files":[],"complete":true}}"#),
    )
    .unwrap();
}

/// A graph with nested, non-ASCII and `graph/` paths, its one launch zip,
/// and `{graph-relative path: sha256}` of every file the zip must hold
/// besides the manifest.
fn format_graph(tag: &str) -> (Graph, PathBuf, std::collections::BTreeMap<String, String>) {
    let g = Graph::new(tag);
    write(&g.graph.join("pages/Příliš/žluťoučký kůň.md"), "- kůň\n");
    write(&g.graph.join("graph/Inside.md"), "- a folder named graph\n");
    write(&g.graph.join("journals/2026_10_11.md"), "- j\n");
    write(&g.graph.join("assets/pic.edn"), "{:w 1}\n");
    write(&g.graph.join("assets/pic.png"), "png");
    write(&g.graph.join("logseq/config.edn"), "{}\n");
    let outcome = g.snap("", BACKUP_KEEP_DEFAULT);
    assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
    assert_eq!(outcome.copied, 6);
    let names = g.zip_names();
    assert_eq!(names.len(), 1, "{names:?}");
    let path = g.zips().join(&names[0]);
    let expected = [
        "assets/pic.edn",
        "graph/Inside.md",
        "journals/2026_10_11.md",
        "logseq/config.edn",
        "pages/A.md",
        "pages/Příliš/žluťoučký kůň.md",
    ]
    .into_iter()
    .map(|rel| {
        let bytes = std::fs::read(g.graph.join(rel)).unwrap();
        (rel.to_owned(), format!("{:x}", Sha256::digest(bytes)))
    })
    .collect();
    (g, path, expected)
}

/// Format (dossier item 1): one zip holding every backed-up file at its
/// graph-relative path, which an independent reader (Python `zipfile`)
/// extracts byte-identical, nested, non-ASCII and a real `graph/` folder
/// included; the manifest is the last entry.
#[test]
fn a_launch_backup_is_one_zip_an_independent_reader_opens() {
    let (g, path, mut expected) = format_graph("backup-zip-format");
    let manifest = verified(&path).manifest;
    assert_eq!(manifest.schema, archive::ZIP_SCHEMA);
    assert_eq!(manifest.writer, SNAPSHOT_WRITER);
    assert_eq!(manifest.root, g.source.root.display().to_string());
    let python = archive::tests::python_zip_digests(&path);
    let manifest_sha = &python[archive::MANIFEST_NAME];
    expected.insert(archive::MANIFEST_NAME.into(), manifest_sha.clone());
    assert_eq!(python, expected);
    g.close();
}

/// "Opens without Tine" with the platform's own extractor (REVIEW gate 6):
/// extract a launch zip with `extract(zip, dest)` and compare every file,
/// by NFC graph-relative path (APFS may store decomposed names), with the
/// graph's bytes. Runs where the extractor exists; it never skips.
#[cfg(any(windows, target_os = "macos"))]
fn the_os_extractor_opens_a_backup(
    tag: &str,
    extract: impl Fn(&std::path::Path, &std::path::Path) -> std::process::Output,
) {
    use unicode_normalization::UnicodeNormalization;
    fn walk(
        dir: &std::path::Path,
        rel: &str,
        out: &mut std::collections::BTreeMap<String, String>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name: String = entry.file_name().to_string_lossy().nfc().collect();
            let rel = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &rel, out);
            } else {
                let bytes = std::fs::read(entry.path()).unwrap();
                out.insert(rel, format!("{:x}", Sha256::digest(bytes)));
            }
        }
    }
    let (g, zip, expected) = format_graph(tag);
    let dest = g.root.join("extracted");
    let output = extract(&zip, &dest);
    assert!(
        output.status.success(),
        "extractor failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut extracted = std::collections::BTreeMap::new();
    walk(&dest, "", &mut extracted);
    let manifest = extracted
        .remove(archive::MANIFEST_NAME)
        .expect("the manifest is extracted too");
    assert!(!manifest.is_empty());
    assert_eq!(extracted, expected);
    g.close();
}

/// Windows: PowerShell's `Expand-Archive` (.NET's zip reader, what
/// Explorer users reach for) extracts the backup byte-identical.
#[cfg(windows)]
#[test]
fn windows_expand_archive_opens_a_backup() {
    the_os_extractor_opens_a_backup("backup-zip-expand-archive", |zip, dest| {
        let quote = |path: &std::path::Path| path.display().to_string().replace('\'', "''");
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "$ErrorActionPreference = 'Stop'; Expand-Archive -LiteralPath '{}' -DestinationPath '{}'",
                    quote(zip),
                    quote(dest)
                ),
            ])
            .output()
            .expect("powershell runs")
    });
}

/// macOS: `ditto -x -k` (the Archive Utility's engine) extracts the backup
/// byte-identical.
#[cfg(target_os = "macos")]
#[test]
fn macos_ditto_opens_a_backup() {
    the_os_extractor_opens_a_backup("backup-zip-ditto", |zip, dest| {
        std::process::Command::new("ditto")
            .arg("-x")
            .arg("-k")
            .arg(zip)
            .arg(dest)
            .output()
            .expect("ditto runs")
    });
}

/// macOS prunes only on a strong directory witness: a directory under the
/// user's home (APFS, where app data lives) must report `Durable`, or launch
/// backups there would accumulate forever.
#[cfg(target_os = "macos")]
#[test]
fn macos_home_directory_sync_is_a_strong_witness() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let dir = home.join(format!(".tine-dir-sync-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let witness = tine_store::directory_durability::sync_directory_witness(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        witness.unwrap(),
        tine_store::directory_durability::DirectoryWitness::Durable
    );
}

/// Item 3 + B2: steady state is one file sync and one directory sync, with
/// the zip synced before its rename and the rename before the directory
/// sync; the namespace's first zip also syncs `backups/` and app data.
/// Item 4: an unchanged launch writes and syncs nothing.
#[test]
fn publication_is_durable_before_visible_and_an_unchanged_launch_writes_nothing() {
    let g = Graph::new("backup-zip-publication");
    ops();
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    assert_eq!(
        ops(),
        ["zip_sync", "zip_rename", "zips_dir_sync", "backups_dir_sync", "app_data_dir_sync"],
        "I-1/I-2: a zip is synced, renamed no-replace, then its directory chain synced; exemplar backup.rs publish_zip"
    );
    let baseline = std::fs::read(g.newest_zip()).unwrap();
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    assert_eq!(
        ops(),
        [
            "baseline_sync",
            "zips_dir_sync",
            "backups_dir_sync",
            "app_data_dir_sync"
        ],
        "D2: an unchanged launch writes no file; it settles its baseline again"
    );
    assert_eq!(g.zip_names().len(), 1);
    assert_eq!(std::fs::read(g.newest_zip()).unwrap(), baseline);
    write(&g.graph.join("pages/A.md"), "- a, edited\n");
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    assert_eq!(
        ops(),
        [
            "zip_sync",
            "zip_rename",
            "zips_dir_sync",
            "backups_dir_sync",
            "app_data_dir_sync"
        ],
        "D1: every publication syncs the whole chain"
    );
    assert_eq!(g.zip_names().len(), 2);
    // A suffixed snapshot is always written, unchanged or not.
    assert!(g
        .snap("pre-journal-rename", BACKUP_KEEP_DEFAULT)
        .failure
        .is_none());
    assert_eq!(g.zip_names().len(), 3);
    g.close();
}

/// R5: the baseline is the newest state record of any suffix, so a launch
/// after A -> B -> A writes A again (it differs from B), and a launch equal
/// to a newer pre-restore snapshot writes nothing.
#[test]
fn the_skip_compares_with_the_newest_snapshot_of_any_suffix() {
    let g = Graph::new("backup-zip-aba");
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    write(&g.graph.join("pages/A.md"), "- b\n");
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    write(&g.graph.join("pages/A.md"), "- a\n");
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    assert_eq!(g.zip_names().len(), 3, "A -> B -> A keeps three states");
    write(&g.graph.join("pages/A.md"), "- c\n");
    assert!(g.snap("pre-restore", BACKUP_KEEP_DEFAULT).failure.is_none());
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    assert_eq!(
        g.zip_names().len(),
        4,
        "the pre-restore snapshot is the baseline"
    );
    g.close();
}

/// Gate 3: the baseline must be this graph's. A verified zip of another
/// graph root with identical file bytes, newest in this namespace (a copied
/// app-data folder), is no reason to skip.
#[test]
fn another_roots_identical_zip_is_never_the_baseline() {
    let mine = Graph::new("backup-zip-root-mine");
    let other = Graph::new("backup-zip-root-other");
    assert!(other.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    std::fs::create_dir_all(mine.zips()).unwrap();
    std::fs::copy(
        other.newest_zip(),
        mine.zips().join("2099-01-01_00-00-00.zip"),
    )
    .unwrap();
    assert!(mine.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    assert_eq!(mine.zip_names().len(), 2, "{:?}", mine.zip_names());
    mine.close();
    other.close();
}

/// B3: an unchanged launch skips only after fully verifying the baseline;
/// a baseline whose payload is damaged (its listing still reads ok) gets a
/// fresh zip.
#[test]
fn a_damaged_baseline_is_never_the_reason_to_skip() {
    let g = Graph::new("backup-zip-damaged-baseline");
    write(
        &g.graph.join("pages/A.md"),
        &"- a stored payload\n".repeat(200),
    );
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    let path = g.zips().join(&g.zip_names()[0]);
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes.windows(10).position(|w| w == b"pages/A.md").unwrap() + 10 + 4;
    bytes[at] ^= 0x01;
    std::fs::write(&path, &bytes).unwrap();
    assert!(
        zip_listing(&path).is_ok(),
        "the damage is in the payload only"
    );
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    assert_eq!(
        g.zip_names().len(),
        2,
        "a fresh zip replaces the damaged baseline"
    );
    g.close();
}

/// Crash matrix (dossier gate): a crash before the rename leaves only a
/// partial, never listed and removed by the next lock holder; a crash after
/// it leaves a complete zip that verifies and lists.
#[test]
fn a_crash_leaves_a_partial_or_a_complete_zip() {
    let g = Graph::new("backup-zip-crash");
    hook(
        "zip_rename",
        Hook::Run(Box::new(|| panic!("crash before rename"))),
    );
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        g.snap("", BACKUP_KEEP_DEFAULT)
    }));
    assert!(crashed.is_err());
    let names = g.zip_names();
    assert_eq!(names.len(), 1);
    assert!(names[0].starts_with(naming::PARTIAL_PREFIX), "{names:?}");
    assert!(list_backups_from_base(&g.base, &g.graph).is_empty());
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    let names = g.zip_names();
    assert_eq!(names.len(), 1);
    assert!(
        !names[0].starts_with(naming::PARTIAL_PREFIX),
        "the partial was cleaned: {names:?}"
    );

    write(&g.graph.join("pages/A.md"), "- crash after rename\n");
    hook(
        "zips_dir_sync",
        Hook::Run(Box::new(|| panic!("crash after rename"))),
    );
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        g.snap("", BACKUP_KEEP_DEFAULT)
    }));
    assert!(crashed.is_err());
    let listed = list_backups_from_base(&g.base, &g.graph);
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].status, "ok");
    let newest = g.newest_zip();
    let files = verified(&newest).files;
    assert!(files.iter().any(|bytes| bytes == b"- crash after rename\n"));
    g.close();
}

/// Gate 1 (REVIEW-backup-zip): a failure, a crash or a weak directory
/// witness at each publication step of a namespace's first zip. The prior
/// snapshot (here a legacy directory) survives every unsettled or failed
/// replacement; recovery (a Settings prune, then a restore's safety
/// snapshot) retires it only after certifying a keeper through app data.
#[test]
fn every_publication_boundary_keeps_the_fallback_until_a_keeper_settles() {
    let ours = format!(r#""writer":"{SNAPSHOT_WRITER}","#);
    let steps = [
        "partial_create",
        "zip_sync",
        "zip_rename",
        "zips_dir_sync",
        "backups_dir_sync",
        "app_data_dir_sync",
    ];
    for (at, step) in steps.into_iter().enumerate() {
        let renamed = at > 2;
        for mode in ["fail", "crash", "weak"] {
            if mode == "weak" && !renamed {
                continue; // only directory syncs have a witness
            }
            let case = format!("{step}/{mode}");
            let g = Graph::new(&format!("backup-zip-boundary-{step}-{mode}"));
            legacy_dir(&g.base, "2026-09-01_00-00-00", SNAPSHOT_SCHEMA, &ours);
            let fallback = g.base.join("2026-09-01_00-00-00");
            hook(
                step,
                match mode {
                    "fail" => Hook::Fail(ErrorKind::Other),
                    "crash" => Hook::Run(Box::new(|| panic!("crash"))),
                    _ => Hook::Weak,
                },
            );
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.snap("", 1)));
            match mode {
                "crash" => assert!(outcome.is_err(), "{case}"),
                "fail" => {
                    let phase = if renamed { "publish-sync" } else { "publish" };
                    assert_eq!(outcome.unwrap().failure.unwrap().phase, phase, "{case}");
                }
                _ => {
                    let outcome = outcome.unwrap();
                    assert!(outcome.failure.is_none(), "{case}");
                    assert_eq!(outcome.durability, Durability::Weak, "{case}");
                }
            }
            assert!(fallback.is_dir(), "{case}: the fallback survives");
            ops();
            prune_now(&g.base, 1, std::time::Duration::from_secs(30));
            if renamed {
                assert_eq!(
                    ops(),
                    [
                        "keeper_sync",
                        "zips_dir_sync",
                        "backups_dir_sync",
                        "app_data_dir_sync",
                        "prune_delete"
                    ],
                    "{case}: the keeper settles through app data before any deletion"
                );
                assert!(!fallback.exists(), "{case}");
            } else {
                assert_eq!(ops(), Vec::<&str>::new(), "{case}: no keeper, no deletion");
                assert!(fallback.is_dir(), "{case}");
            }
            assert!(g.snap("pre-restore", 1).failure.is_none(), "{case}");
            assert!(
                g.zip_names()
                    .iter()
                    .all(|name| !name.starts_with(naming::PARTIAL_PREFIX)),
                "{case}: {:?}",
                g.zip_names()
            );
            assert_eq!(fallback.is_dir(), !renamed, "{case}");
            assert!(list_backups_from_base(&g.base, &g.graph)
                .iter()
                .all(|info| info.status == "ok"));
            g.close();
        }
    }
}

/// Gate 2 helper: another Tine process's launch backup, spawned by
/// `two_processes_exclude_each_other_and_a_dead_writer_is_recovered`. It
/// pauses holding the namespace lock, its partial synced, just before the
/// rename, until a line arrives on stdin.
#[test]
fn backup_writer_child() {
    let Some(graph) = std::env::var_os("TINE_BACKUP_CHILD_GRAPH").map(PathBuf::from) else {
        return;
    };
    let base = PathBuf::from(std::env::var_os("TINE_BACKUP_CHILD_BASE").unwrap());
    let refuse = std::env::var_os("TINE_BACKUP_CHILD_REFUSE_NOREPLACE").is_some();
    crate::device_io::REFUSE_NOREPLACE_FLAG.with(|flag| flag.set(refuse));
    let store = Store::open(&graph, Default::default()).unwrap().0;
    let source = BackupSource::from_store(&store, &graph).unwrap();
    hook(
        "zip_rename",
        Hook::Run(Box::new(|| {
            use std::io::{BufRead, Write};
            println!("CHILD:paused");
            std::io::stdout().flush().unwrap();
            let mut line = String::new();
            let _ = std::io::stdin().lock().read_line(&mut line);
        })),
    );
    let outcome = write_snapshot(
        &base,
        &store,
        source,
        "",
        &|| false,
        1,
        std::time::Duration::from_secs(30),
    );
    println!(
        "CHILD:done {:?}",
        outcome.failure.map(|failure| failure.phase)
    );
    store.close();
}

/// Gate 2 (REVIEW-backup-zip): two OS processes. While one holds the
/// namespace lock mid-publication, the other's launch is busy, its Settings
/// prune deletes nothing and its restore safety snapshot is busy; the live
/// partial is left alone. Released, the holder publishes (also through the
/// no-replace fallback) and prunes, and the other's identical capture is
/// unchanged. Killed, its lock passes on and the next holder removes the
/// dead partial and publishes.
#[test]
fn two_processes_exclude_each_other_and_a_dead_writer_is_recovered() {
    use std::io::{BufRead, Write};
    for (refuse, kill) in [(false, false), (true, false), (false, true)] {
        let case = format!("refuse={refuse} kill={kill}");
        let g = Graph::new(&format!("backup-zip-two-process-{refuse}-{kill}"));
        for text in ["- 1\n", "- 2\n"] {
            write(&g.graph.join("pages/A.md"), text);
            assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
        }
        let before = g.zip_names();
        write(&g.graph.join("pages/A.md"), "- 3\n");
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "backup::tests::backup_writer_child",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TINE_BACKUP_CHILD_GRAPH", &g.graph)
            .env("TINE_BACKUP_CHILD_BASE", &g.base)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        if refuse {
            command.env("TINE_BACKUP_CHILD_REFUSE_NOREPLACE", "1");
        }
        let mut child = command.spawn().unwrap();
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut until = |marker: &str| {
            let mut line = String::new();
            while !line.contains(marker) {
                line.clear();
                assert_ne!(out.read_line(&mut line).unwrap(), 0, "{case}: child exited");
            }
            line
        };
        until("CHILD:paused");
        let partials: Vec<String> = g
            .zip_names()
            .into_iter()
            .filter(|name| name.starts_with(naming::PARTIAL_PREFIX))
            .collect();
        assert_eq!(partials.len(), 1, "{case}");
        let short = std::time::Duration::from_millis(200);
        let busy = g.snap_waiting("", 1, short).failure.map(|f| f.phase);
        assert_eq!(busy, Some("busy"), "{case}");
        prune_now(&g.base, 1, short);
        let safety = g
            .snap_waiting("pre-restore", 1, short)
            .failure
            .map(|f| f.phase);
        assert_eq!(safety, Some("busy"), "{case}");
        let mut held = before.clone();
        held.extend(partials.iter().cloned());
        held.sort();
        assert_eq!(g.zip_names(), held, "{case}: nothing deleted while held");
        if kill {
            child.kill().unwrap();
            child.wait().unwrap();
            assert!(g.zips().join(&partials[0]).exists(), "{case}");
            assert!(g.snap("", 1).failure.is_none(), "{case}");
        } else {
            writeln!(child.stdin.take().unwrap()).unwrap();
            assert!(until("CHILD:done").contains("CHILD:done None"), "{case}");
            child.wait().unwrap();
            ops();
            assert!(g.snap("", 1).failure.is_none(), "{case}");
            assert_eq!(
                ops(),
                [
                    "baseline_sync",
                    "zips_dir_sync",
                    "backups_dir_sync",
                    "app_data_dir_sync"
                ],
                "{case}: the identical capture is unchanged; its baseline is settled again"
            );
        }
        let names = g.zip_names();
        assert_eq!(
            names.len(),
            1,
            "{case}: keep=1 after a settled keeper: {names:?}"
        );
        assert!(!before.contains(&names[0]), "{case}");
        assert!(
            verified(&g.newest_zip())
                .files
                .iter()
                .any(|bytes| bytes == b"- 3\n"),
            "{case}"
        );
        g.close();
    }
}

/// I/O failures: before the rename the partial is removed and the backup
/// fails `publish`; after it the zip stays and the backup fails
/// `publish-sync`, deleting nothing.
#[test]
fn publication_failures_name_their_phase_and_delete_nothing() {
    let g = Graph::new("backup-zip-failures");
    hook("zip_sync", Hook::Fail(ErrorKind::Other));
    let failure = g.snap("", BACKUP_KEEP_DEFAULT).failure.unwrap();
    assert_eq!((failure.phase, failure.kind), ("publish", ErrorKind::Other));
    assert_eq!(g.zip_names(), Vec::<String>::new());
    assert!(g.snap("", 1).failure.is_none());
    write(&g.graph.join("pages/A.md"), "- b\n");
    hook("zips_dir_sync", Hook::Fail(ErrorKind::Other));
    let failure = g.snap("", 1).failure.unwrap();
    assert_eq!(failure.phase, "publish-sync");
    assert_eq!(g.zip_names().len(), 2, "no prune after a failed sync");
    g.close();
}

/// R1 + keeper rule: a weak directory witness publishes but prunes
/// nothing. The keeper is the newest zip that fully verifies (D2): a
/// payload-damaged newest zip is never it, and holds no slot; the newest
/// healthy zip is settled before anything goes.
#[test]
fn deletion_needs_a_verified_strongly_synced_keeper() {
    let g = Graph::new("backup-zip-keeper");
    for text in ["- 1\n", "- 2\n"] {
        write(&g.graph.join("pages/A.md"), text);
        assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    }
    write(&g.graph.join("pages/A.md"), "- 3\n");
    hook("zips_dir_sync", Hook::Weak);
    let weak = g.snap("", 1);
    assert!(weak.failure.is_none());
    assert_eq!(weak.durability, Durability::Weak, "D1: callers see it");
    assert_eq!(g.zip_names().len(), 3, "weak: nothing retired");

    // The newest zip's payload is damaged (listing ok).
    let damaged = g.newest_zip();
    let mut bytes = std::fs::read(&damaged).unwrap();
    let at = bytes.windows(10).position(|w| w == b"pages/A.md").unwrap() + 10 + 2;
    bytes[at] ^= 0x01;
    std::fs::write(&damaged, &bytes).unwrap();
    assert!(zip_listing(&damaged).is_ok());
    ops();
    prune_now(&g.base, 1, std::time::Duration::from_secs(30));
    assert_eq!(
        ops(),
        [
            "keeper_sync",
            "zips_dir_sync",
            "backups_dir_sync",
            "app_data_dir_sync",
            "prune_delete"
        ],
        "the newest healthy zip is settled as keeper before the excess goes"
    );
    assert_eq!(g.zip_names().len(), 2, "{:?}", g.zip_names());
    assert!(
        damaged.exists(),
        "newer than the oldest retained healthy zip"
    );
    // A weak publication retires nothing.
    write(&g.graph.join("pages/A.md"), "- 4\n");
    hook("app_data_dir_sync", Hook::Weak);
    ops();
    assert!(g.snap("", 1).failure.is_none());
    assert_eq!(g.zip_names().len(), 3);
    assert_eq!(
        ops(),
        [
            "zip_sync",
            "zip_rename",
            "zips_dir_sync",
            "backups_dir_sync",
            "app_data_dir_sync"
        ],
        "the fresh keeper is certified through app data before any deletion"
    );
    // A Settings prune re-syncs the keeper's file and whole chain first.
    prune_now(&g.base, 1, std::time::Duration::from_secs(30));
    let ops = ops();
    assert_eq!(
        &ops[..5],
        [
            "keeper_sync",
            "zips_dir_sync",
            "backups_dir_sync",
            "app_data_dir_sync",
            "prune_delete"
        ]
    );
    assert_eq!(g.zip_names().len(), 1);
    g.close();
}

/// D2: an unverified zip is never the keeper, and without a verified zip
/// nothing is deleted, not even legacy snapshots beyond the keep count.
#[test]
fn without_a_verified_zip_nothing_is_deleted() {
    let g = Graph::new("backup-zip-no-keeper");
    let ours = format!(r#""writer":"{SNAPSHOT_WRITER}","#);
    legacy_dir(&g.base, "2026-09-01_00-00-00", SNAPSHOT_SCHEMA, &ours);
    legacy_dir(&g.base, "2026-09-02_00-00-00", SNAPSHOT_SCHEMA, &ours);
    write(
        &g.graph.join("pages/A.md"),
        &"- a stored payload\n".repeat(200),
    );
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    damage_payload(&g.newest_zip());
    let before = g.all_names();
    ops();
    prune_now(&g.base, 1, std::time::Duration::from_secs(30));
    assert_eq!(ops(), Vec::<&str>::new());
    assert_eq!(g.all_names(), before);
    g.close();
}

/// Item 5 + R5: keep=N counts legacy directories and zips together, oldest
/// first; `-pre-restore` is the only exemption (a pre-journal-rename
/// snapshot counts); another Tine's snapshots, unknown names and
/// unsupported zips are never touched; a damaged zip goes only once older
/// than the oldest retained healthy zip.
#[test]
fn keep_n_prunes_both_namespaces_with_the_exemptions() {
    let g = Graph::new("backup-zip-prune");
    let ours = format!(r#""writer":"{SNAPSHOT_WRITER}","#);
    legacy_dir(&g.base, "2026-09-01_00-00-00", SNAPSHOT_SCHEMA, &ours);
    legacy_dir(
        &g.base,
        "2026-09-02_00-00-00",
        SNAPSHOT_SCHEMA,
        r#""writer":"master","#,
    );
    legacy_dir(&g.base, "2026-09-03_00-00-00", LEGACY_SNAPSHOT_SCHEMA, "");
    legacy_dir(
        &g.base,
        "2026-09-04_00-00-00-pre-restore",
        SNAPSHOT_SCHEMA,
        &ours,
    );
    legacy_dir(&g.base, "notes", SNAPSHOT_SCHEMA, &ours);
    std::fs::create_dir_all(g.zips()).unwrap();
    std::fs::write(g.zips().join("2026-09-05_00-00-00.zip"), b"torn").unwrap();
    std::fs::write(g.zips().join("unknown.zip"), b"?").unwrap();
    assert!(g.snap("pre-restore", BACKUP_KEEP_DEFAULT).failure.is_none());
    write(&g.graph.join("pages/A.md"), "- 2\n");
    assert!(g
        .snap("pre-journal-rename", BACKUP_KEEP_DEFAULT)
        .failure
        .is_none());
    write(&g.graph.join("pages/A.md"), "- 3\n");
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    let zips = g.zip_names();
    let [pre_restore, rename, launch] = [
        zips.iter()
            .find(|n| n.contains("pre-restore"))
            .unwrap()
            .clone(),
        zips.iter()
            .find(|n| n.contains("pre-journal-rename"))
            .unwrap()
            .clone(),
        zips.iter()
            .find(|n| !n.contains("pre-") && !n.starts_with("2026-09") && *n != "unknown.zip")
            .unwrap()
            .clone(),
    ];
    // Counted, oldest first: 09-01, 09-03, rename, launch. keep=3 retires 09-01;
    // the torn zip is older than the oldest retained healthy zip.
    prune_now(&g.base, 3, std::time::Duration::from_secs(30));
    let mut expected = vec![
        "2026-09-02_00-00-00".to_owned(),
        "2026-09-03_00-00-00".into(),
        "2026-09-04_00-00-00-pre-restore".into(),
        "notes".into(),
        pre_restore.clone(),
        rename.clone(),
        launch.clone(),
        "unknown.zip".into(),
    ];
    expected.sort();
    assert_eq!(g.all_names(), expected);
    prune_now(&g.base, 1, std::time::Duration::from_secs(30));
    let mut expected = vec![
        "2026-09-02_00-00-00".to_owned(),
        "2026-09-04_00-00-00-pre-restore".into(),
        "notes".into(),
        pre_restore,
        launch,
        "unknown.zip".into(),
    ];
    expected.sort();
    assert_eq!(g.all_names(), expected);
    g.close();
}

/// Flip one payload byte of `pages/A.md` inside `zip` (its listing still
/// reads ok). `A.md` must hold a payload of a few hundred bytes.
fn damage_payload(zip: &std::path::Path) {
    let mut bytes = std::fs::read(zip).unwrap();
    let at = bytes.windows(10).position(|w| w == b"pages/A.md").unwrap() + 10 + 4;
    bytes[at] ^= 0x01;
    std::fs::write(zip, &bytes).unwrap();
    assert!(
        zip_listing(zip).is_ok(),
        "the damage is in the payload only"
    );
}

/// D1 (REVIEW-backup-zip F1, plant 2): no publication infers its ancestors
/// settled from an earlier zip's presence. The first zip's `backups/` sync
/// fails; the next publication still syncs file, `<id>.zips/`, `backups/`
/// and app data before it counts as settled.
#[test]
fn every_publication_settles_the_whole_chain() {
    let g = Graph::new("backup-zip-chain-every-time");
    hook("backups_dir_sync", Hook::Fail(ErrorKind::Other));
    assert_eq!(
        g.snap("", BACKUP_KEEP_DEFAULT).failure.unwrap().phase,
        "publish-sync"
    );
    write(&g.graph.join("pages/A.md"), "- edited\n");
    ops();
    let outcome = g.snap("pre-journal-rename", BACKUP_KEEP_DEFAULT);
    assert!(outcome.failure.is_none());
    assert_eq!(outcome.durability, Durability::Strong);
    assert_eq!(
        ops(),
        [
            "zip_sync",
            "zip_rename",
            "zips_dir_sync",
            "backups_dir_sync",
            "app_data_dir_sync"
        ],
        "I-1/I-2: every publication syncs its whole directory chain; exemplar backup.rs publish_zip"
    );
    g.close();
}

/// D2 (F2): a retention slot needs full verification. A middle zip whose
/// payload is damaged (its listing reads ok) counts toward nothing, so
/// keep=2 keeps both healthy zips; the damaged one is not older than the
/// oldest retained healthy zip, so it stays too.
#[test]
fn a_payload_damaged_zip_holds_no_retention_slot() {
    let g = Graph::new("backup-zip-damaged-middle");
    let mut zips = Vec::new();
    for text in [
        "- a\n".to_owned(),
        "- b stored payload\n".repeat(200),
        "- c\n".into(),
    ] {
        write(&g.graph.join("pages/A.md"), &text);
        assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
        zips.push(g.newest_zip());
    }
    damage_payload(&zips[1]);
    prune_now(&g.base, 2, std::time::Duration::from_secs(30));
    assert!(zips[0].exists(), "A, healthy, holds the second slot");
    assert!(zips[2].exists(), "C, healthy, holds the first slot");
    assert!(zips[1].exists(), "B is newer than the oldest retained zip");
    g.close();
}

/// D2 (F2): the pre-restore exemption covers a damaged archive too.
#[test]
fn a_damaged_pre_restore_zip_is_never_pruned() {
    let g = Graph::new("backup-zip-damaged-pre-restore");
    assert!(g.snap("pre-restore", BACKUP_KEEP_DEFAULT).failure.is_none());
    let pre_restore = g.newest_zip();
    std::fs::write(&pre_restore, b"torn").unwrap();
    for text in ["- 2\n", "- 3\n"] {
        write(&g.graph.join("pages/A.md"), text);
        assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    }
    prune_now(&g.base, 1, std::time::Duration::from_secs(30));
    assert!(pre_restore.exists(), "{:?}", g.zip_names());
    assert_eq!(g.zip_names().len(), 2, "{:?}", g.zip_names());
    g.close();
}

/// D2 (F4): every locked path applies the keep count. A Settings prune
/// that found the namespace busy is applied by the next launch, unchanged
/// graph or not.
#[test]
fn an_unchanged_launch_applies_a_keep_count_a_busy_prune_left() {
    let g = Graph::new("backup-zip-busy-settings");
    for text in ["- 1\n", "- 2\n", "- 3\n"] {
        write(&g.graph.join("pages/A.md"), text);
        assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    }
    let held = crate::file_lock::lock_exclusive_until(
        &g.zips().join(naming::LOCK_FILE),
        std::time::Instant::now(),
    )
    .unwrap()
    .unwrap();
    prune_now(&g.base, 1, std::time::Duration::ZERO);
    assert_eq!(g.zip_names().len(), 3, "busy: nothing pruned");
    drop(held);
    assert!(g.snap("", 1).failure.is_none());
    assert_eq!(g.zip_names().len(), 1, "{:?}", g.zip_names());
    g.close();
}

/// D2 (F4): a deletion that failed is retried by the next unchanged launch.
#[test]
fn a_failed_prune_delete_is_retried_by_the_next_unchanged_launch() {
    let g = Graph::new("backup-zip-retry-delete");
    for text in ["- 1\n", "- 2\n"] {
        write(&g.graph.join("pages/A.md"), text);
        assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    }
    write(&g.graph.join("pages/A.md"), "- 3\n");
    hook("prune_delete", Hook::Fail(ErrorKind::PermissionDenied));
    assert!(g.snap("", 1).failure.is_none());
    assert_eq!(g.zip_names().len(), 2, "one deletion failed");
    assert!(g.snap("", 1).failure.is_none());
    assert_eq!(g.zip_names().len(), 1, "{:?}", g.zip_names());
    g.close();
}

/// D4 (F5): a partial's name is reserved before it is cleaned. A partial
/// left at a later stamp (the clock then moved backward) still sorts every
/// new snapshot after it.
#[test]
fn an_abandoned_partial_still_reserves_its_name() {
    let g = Graph::new("backup-zip-partial-reserves");
    std::fs::create_dir_all(g.zips()).unwrap();
    std::fs::write(
        g.zips()
            .join(".partial-2099-01-01_00-00-00-pre-restore-77.zip"),
        b"abandoned",
    )
    .unwrap();
    assert!(g
        .snap("pre-journal-rename", BACKUP_KEEP_DEFAULT)
        .failure
        .is_none());
    assert_eq!(
        g.zip_names(),
        ["2099-01-01_00-00-00-pre-journal-rename-78.zip"]
    );
    g.close();
}

/// D3 (REVIEW-backup-zip F3): capture spends the archive's limits while
/// reading. Past the aggregate payload limit or the entry count the
/// capture is refused in its phase, holding at most what fit.
#[test]
fn capture_refuses_at_the_aggregate_limits_while_reading() {
    let g = Graph::new("backup-zip-capture-budget");
    write(&g.graph.join("pages/B.md"), &"- b\n".repeat(100));
    write(&g.graph.join("assets/pic.edn"), "{:a 1}\n");
    for (budget, phase) in [
        (
            Budget {
                bytes: 100,
                entries: 10,
            },
            "graph",
        ),
        (
            Budget {
                bytes: 1 << 20,
                entries: 1,
            },
            "graph",
        ),
        (
            Budget {
                bytes: 404,
                entries: 10,
            },
            "assets",
        ),
        (
            Budget {
                bytes: 1 << 20,
                entries: 2,
            },
            "assets",
        ),
    ] {
        let label = format!("{}/{}", budget.bytes, budget.entries);
        let Err(outcome) = capture(&g.store, g.source.clone(), &|| false, budget) else {
            panic!("{label}: refused");
        };
        let failure = outcome.failure.unwrap();
        assert_eq!(
            (failure.phase, failure.kind),
            (phase, ErrorKind::FileTooLarge),
            "{label}"
        );
    }
    let fits = capture(
        &g.store,
        g.source.clone(),
        &|| false,
        Budget {
            bytes: 411,
            entries: 3,
        },
    );
    assert_eq!(fits.ok().unwrap().files.len(), 3);
    g.close();
}

/// D8 (REVIEW-backup-zip F9): restore probes the graph volume's name
/// folding instead of assuming the platform's. Linux test volumes tell case
/// apart; the Windows and macOS runners' default volumes fold it (Apple
/// also folds Unicode normalization). A root with no entry to probe counts
/// as folding.
#[test]
fn restore_probes_the_destination_volume_folding() {
    let root = scratch("backup-fold-probe");
    let empty = restore::destination_fold(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    let probed = restore::destination_fold(&root);
    let (folding, exact) = if cfg!(any(target_os = "macos", target_os = "ios")) {
        (
            archive::Fold::CaseAndNormalization,
            archive::Fold::Normalization,
        )
    } else {
        (archive::Fold::Case, archive::Fold::Exact)
    };
    assert_eq!(empty, folding, "nothing to probe: the refusing side");
    if cfg!(target_os = "linux") {
        assert_eq!(probed, exact);
    } else if cfg!(any(windows, target_os = "macos")) {
        assert_eq!(probed, folding);
    }
    let _ = std::fs::remove_dir_all(root);
}

/// D7 (REVIEW-backup-zip F8): the production publication, settle and prune
/// paths, each after a marker (`unlink` of a path that cannot exist) the
/// syscall gate splits its trace on. `scripts/check-backup-syscalls.mjs`
/// runs this test under `strace` and checks the order of syncs, renames
/// and unlinks; alone it only checks the outcomes.
#[test]
fn syscall_gate_scenarios() {
    let mark = |scenario: &str| {
        let _ = std::fs::remove_file(format!("/tine-syscall-gate-marker/{scenario}"));
    };
    let g = Graph::new("backup-syscall-gate");
    let mut edit = 0;
    let mut snap = |marker: &str, keep: usize, changed: bool| {
        if changed {
            edit += 1;
            write(&g.graph.join("pages/A.md"), &format!("- {edit}\n"));
        }
        mark(marker);
        let outcome = g.snap("", keep);
        assert!(outcome.failure.is_none(), "{marker}: {:?}", outcome.failure);
    };
    snap("first-publication", 2, true);
    snap("unchanged-settled", 2, false);
    snap("changed-publication", 2, true);
    snap("publication-with-prune", 2, true);
    assert_eq!(g.zip_names().len(), 2);
    snap("publication-keep-12", BACKUP_KEEP_DEFAULT, true);
    mark("settings-prune");
    prune_now(&g.base, 1, std::time::Duration::from_secs(30));
    assert_eq!(g.zip_names().len(), 1);
    mark("end");
    g.close();
}

/// B1/R2/R3: concurrent honest writers. A second writer cannot enter while
/// the first holds the namespace lock (a bounded wait reports `busy`, which
/// a launch skips silently); one that waits publishes after it under a
/// distinct, later name, and its prune keeps exactly the newest. Also with
/// the no-replace flag refused (GH #538 storage), whose check-then-rename
/// fallback is safe because it runs under the lock.
#[test]
fn concurrent_writers_serialize_on_the_namespace_lock() {
    for refuse_flag in [false, true] {
        let g = std::sync::Arc::new(Graph::new(&format!("backup-zip-concurrent-{refuse_flag}")));
        let other = g.clone();
        let (busy_tx, busy_rx) = std::sync::mpsc::channel();
        let (waiter_tx, waiter_rx) = std::sync::mpsc::channel();
        hook(
            "zip_rename",
            Hook::Run(Box::new(move || {
                let impatient = other.clone();
                let busy = std::thread::spawn(move || {
                    impatient.snap_waiting("pre-journal-rename", 1, std::time::Duration::ZERO)
                })
                .join()
                .unwrap();
                busy_tx
                    .send(busy.failure.map(|failure| failure.phase))
                    .unwrap();
                let patient = other.clone();
                waiter_tx
                    .send(std::thread::spawn(move || {
                        crate::device_io::REFUSE_NOREPLACE_FLAG.with(|flag| flag.set(refuse_flag));
                        patient.snap("pre-journal-rename", 1)
                    }))
                    .unwrap();
            })),
        );
        crate::device_io::REFUSE_NOREPLACE_FLAG.with(|flag| flag.set(refuse_flag));
        assert!(g.snap("", 1).failure.is_none());
        crate::device_io::REFUSE_NOREPLACE_FLAG.with(|flag| flag.set(false));
        assert_eq!(busy_rx.recv().unwrap(), Some("busy"));
        let busy = BackupOutcome::failed(1, "busy", ErrorKind::WouldBlock);
        assert_eq!(
            launch_failure_token(&busy),
            None,
            "a busy launch is skipped, not a toast"
        );
        let waited = waiter_rx.recv().unwrap().join().unwrap();
        assert!(waited.failure.is_none(), "{:?}", waited.failure);
        let names = g.zip_names();
        assert_eq!(
            names.len(),
            1,
            "keep=1 retired the first writer's zip: {names:?}"
        );
        assert!(names[0].contains("pre-journal-rename"));
        let g = std::sync::Arc::try_unwrap(g).ok().unwrap();
        g.close();
    }
}

/// Item 2: the zip namespace is a sibling of the legacy directory, which
/// builds before the zip format read alone; listing shows both, newest
/// first, with namespace-qualified ids and a status.
#[test]
fn listing_shows_both_namespaces_with_ids_and_status() {
    let g = Graph::new("backup-zip-listing");
    assert_eq!(
        g.zips().file_name().unwrap().to_string_lossy(),
        format!("{}.zips", g.base.file_name().unwrap().to_string_lossy())
    );
    let canonical = g.source.root.display().to_string();
    let snapshot = g.base.join("2026-09-01_00-00-00");
    std::fs::create_dir_all(snapshot.join("pages")).unwrap();
    write_manifest(
        &snapshot,
        &SnapshotManifest {
            schema: LEGACY_SNAPSHOT_SCHEMA,
            root: canonical,
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            graph_text_policy: None,
            writer: None,
            files: Vec::new(),
            complete: true,
        },
    )
    .unwrap();
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    std::fs::write(g.zips().join("2026-09-02_00-00-00.zip"), b"torn").unwrap();
    let listed = list_backups_from_base(&g.base, &g.graph);
    let rows: Vec<(&str, &str)> = listed
        .iter()
        .map(|info| (info.id.as_str(), info.status))
        .collect();
    assert_eq!(rows.len(), 3);
    assert!(
        rows[0].0.starts_with("zip:2026-1") && rows[0].1 == "ok",
        "{rows:?}"
    );
    assert_eq!(
        rows[1..],
        [
            ("zip:2026-09-02_00-00-00", "damaged"),
            ("dir:2026-09-01_00-00-00", "ok")
        ]
    );
    g.close();
}

/// Item 7: listing reads the central directory and the manifest entry,
/// never a payload: a zip whose payload is corrupt still lists as ok (its
/// restore refuses), and no legacy payload is hashed.
#[test]
fn listing_never_reads_snapshot_payloads() {
    let g = Graph::new("backup-zip-cheap-listing");
    let snapshot = g.base.join("2026-07-22_12-00-00");
    std::fs::create_dir_all(snapshot.join("pages")).unwrap();
    std::fs::write(snapshot.join("pages/note.md"), b"tampered payload").unwrap();
    write_manifest(
        &snapshot,
        &SnapshotManifest {
            schema: LEGACY_SNAPSHOT_SCHEMA,
            root: g.source.root.display().to_string(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            graph_text_policy: None,
            writer: None,
            files: vec![SnapshotFile {
                path: "pages/note.md".into(),
                sha256: "manifest metadata only".into(),
            }],
            complete: true,
        },
    )
    .unwrap();
    write(
        &g.graph.join("pages/A.md"),
        &"- a stored payload\n".repeat(200),
    );
    assert!(g.snap("", BACKUP_KEEP_DEFAULT).failure.is_none());
    let zip = g.newest_zip();
    let mut bytes = std::fs::read(&zip).unwrap();
    let at = bytes.windows(10).position(|w| w == b"pages/A.md").unwrap() + 10 + 4;
    bytes[at] ^= 0x01;
    std::fs::write(&zip, &bytes).unwrap();

    PAYLOAD_HASH_READS.with(|reads| reads.set(0));
    let listed = list_backups_from_base(&g.base, &g.graph);
    assert_eq!(
        PAYLOAD_HASH_READS.with(|reads| reads.get()),
        0,
        "listing must not read or hash snapshot payloads"
    );
    assert_eq!(listed.len(), 2);
    assert_eq!((listed[0].files, listed[0].status), (1, "ok"));
    assert_eq!((listed[1].files, listed[1].status), (1, "ok"));
    g.close();
}

/// Launch-backup cost on a real graph (dossier og-backup-zip): files
/// created and bytes written under the backups dir, sync calls, wall time,
/// for a first backup, an unchanged second one, one after three page
/// edits, a repair (the newest zip damaged, graph unchanged) and a prune
/// (one more edit, keep 1); with files deleted, the raw graph bytes the zip
/// holds and the time a full verification of the newest zip takes. Runs
/// only on a scratch COPY of a graph (it edits four pages):
/// `TINE_BACKUP_CORPUS=<copy> cargo test -p tine launch_backup_cost -- --ignored --nocapture`.
#[test]
#[ignore = "measurement; needs TINE_BACKUP_CORPUS (a scratch copy of a graph)"]
fn launch_backup_cost_on_corpus() {
    let graph = PathBuf::from(std::env::var_os("TINE_BACKUP_CORPUS").expect("TINE_BACKUP_CORPUS"));
    let app_data = graph.with_extension("backup-cost-app");
    let _ = std::fs::remove_dir_all(&app_data);
    let backups = app_data.join("backups");
    let files_under = |dir: &std::path::Path| {
        let mut out = std::collections::BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let meta = entry.metadata().unwrap();
                if meta.is_dir() {
                    stack.push(entry.path());
                } else {
                    out.insert(entry.path(), meta.len());
                }
            }
        }
        out
    };
    let run = |label: &str, keep: usize| {
        let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
        let source = BackupSource::from_store(&store, &graph).unwrap();
        let before = files_under(&backups);
        BACKUP_OPS.with(|ops| ops.borrow_mut().clear());
        let started = std::time::Instant::now();
        let base = backups.join(root_backup_id(&source.root));
        let outcome = write_snapshot(&base, &store, source, "", &|| false, keep, BACKUP_LOCK_WAIT);
        let elapsed = started.elapsed();
        assert!(outcome.failure.is_none(), "{:?}", outcome.failure);
        let after = files_under(&backups);
        let created: Vec<_> = after
            .keys()
            .filter(|path| !before.contains_key(*path))
            .collect();
        let bytes: u64 = created.iter().map(|path| after[*path]).sum();
        let deleted = before
            .keys()
            .filter(|path| !after.contains_key(*path))
            .count();
        let ops = BACKUP_OPS.with(|ops| ops.borrow().clone());
        let syncs = ops.iter().filter(|op| op.ends_with("sync")).count();
        let newest = newest_zip_in(&base, &graph);
        let verify_started = std::time::Instant::now();
        let verified = verified(&newest);
        let verify_ms = verify_started.elapsed().as_secs_f64() * 1000.0;
        let raw: u64 = verified.manifest.files.iter().map(|file| file.len).sum();
        eprintln!(
            "BACKUP-COST {label}: graph_files={} created_files={} deleted_files={deleted} bytes_written={bytes} syncs={syncs} wall_ms={:.1} zip_bytes={} raw_bytes={raw} full_verify_ms={verify_ms:.1}",
            outcome.copied,
            created.len(),
            elapsed.as_secs_f64() * 1000.0,
            std::fs::metadata(&newest).unwrap().len(),
        );
        store.close();
    };
    run("first", BACKUP_KEEP_DEFAULT);
    run("unchanged", BACKUP_KEEP_DEFAULT);
    let mut pages: Vec<PathBuf> = std::fs::read_dir(graph.join("pages"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    pages.sort();
    let edit = |page: &PathBuf| {
        let mut text = std::fs::read(page).unwrap();
        text.extend_from_slice(b"\n- backup cost probe edit\n");
        std::fs::write(page, text).unwrap();
    };
    pages[..3].iter().for_each(edit);
    run("three-edits", BACKUP_KEEP_DEFAULT);
    let base = backups.join(root_backup_id(&graph));
    let newest = newest_zip_in(&base, &graph);
    let mut damaged = std::fs::read(&newest).unwrap();
    let middle = damaged.len() / 2;
    damaged[middle] ^= 0x01;
    std::fs::write(&newest, damaged).unwrap();
    run("repair", BACKUP_KEEP_DEFAULT);
    edit(&pages[3]);
    run("prune", 1);
    let _ = std::fs::remove_dir_all(&app_data);
}

/// I-9: an unreadable graph file fails the backup with its phase and
/// cause, which reach the launch diagnostic.
#[cfg(unix)]
#[test]
fn launch_backup_read_failure_reaches_diagnostic_adapter() {
    use std::os::unix::fs::PermissionsExt;
    let g = Graph::new("launch-backup-read-error");
    let page = g.graph.join("pages/A.md");
    std::fs::set_permissions(&page, std::fs::Permissions::from_mode(0o000)).unwrap();
    let outcome = g.snap("", BACKUP_KEEP_DEFAULT);
    std::fs::set_permissions(&page, std::fs::Permissions::from_mode(0o600)).unwrap();
    let failure = outcome.failure.clone().unwrap();
    assert_eq!(
        (failure.phase, failure.kind),
        ("graph", ErrorKind::PermissionDenied)
    );
    let token = launch_failure_token(&outcome).unwrap();
    assert_eq!(token, "backup-failed:graph:PermissionDenied",
        "I-9: forced read failure must reach the launch diagnostic adapter; exemplar backup.rs backup_async");
    assert_eq!(g.zip_names(), Vec::<String>::new(), "nothing published");
    g.close();
}

#[test]
fn backup_root_ids_do_not_conflate_punctuation() {
    let root = scratch("backup-root-id");
    let dash = root.join("a-b");
    let underscore = root.join("a_b");
    std::fs::create_dir_all(&dash).unwrap();
    std::fs::create_dir_all(&underscore).unwrap();
    assert_ne!(root_backup_id(&dash), root_backup_id(&underscore));
    assert_eq!(root_backup_id(&dash), root_backup_id(&dash));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn runtime_backup_reads_graph_files_through_store() {
    let root = scratch("store-backup-read");
    std::fs::create_dir_all(root.join("pages/nested")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages/nested/Note.md"), b"- note\n").unwrap();
    std::fs::write(root.join("pages/Ignore.txt"), b"skip").unwrap();
    let (store, _, _) = Store::open(&root, tine_store::OpenOptions::default()).unwrap();
    let mut out = Vec::new();
    let read = read_store_area(
        &store,
        Area::Pages,
        "pages/",
        is_graph_text,
        &|| false,
        &mut Budget::archive(),
        &mut out,
    );
    assert_eq!(read.unwrap(), 1);
    assert_eq!(
        out,
        [("pages/nested/Note.md".to_owned(), b"- note\n".to_vec())]
    );
    store.close();
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn backup_source_refuses_retargeted_external_assets() {
    let root = scratch("retargeted-backup-assets");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    let first = root.with_extension("assets-first");
    let second = root.with_extension("assets-second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::os::unix::fs::symlink(&first, root.join("assets")).unwrap();
    let (store, _, _) = Store::open(
        &root,
        tine_store::OpenOptions {
            approved_external_assets: Some(first.clone()),
            watch: Default::default(),
            launch_checkpoint: None,
        },
    )
    .unwrap();
    assert!(BackupSource::from_store(&store, &root).is_ok());
    std::fs::remove_file(root.join("assets")).unwrap();
    std::os::unix::fs::symlink(&second, root.join("assets")).unwrap();
    assert!(BackupSource::from_store(&store, &root).is_err());
    store.close();
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(first);
    let _ = std::fs::remove_dir_all(second);
}

/// REG-OG-C5-L06-B1 (I-9): a graph inventory that cannot be read names
/// its cause in the backup token. Before, every inventory failure became
/// `backup-failed:inventory:Other`, hiding a permission or disk error.
#[cfg(unix)]
#[test]
fn inventory_failure_keeps_its_error_kind() {
    use std::os::unix::fs::PermissionsExt;
    let g = Graph::new("inventory-error-kind");
    write(&g.graph.join("pages/locked/a.md"), "- a\n");
    let locked = g.graph.join("pages/locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let outcome = g.snap("", BACKUP_KEEP_DEFAULT);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    let failure = outcome
        .failure
        .expect("an unreadable page directory fails the backup");
    assert_eq!(
        (failure.phase, failure.kind),
        ("inventory", ErrorKind::PermissionDenied),
        "I-9: the inventory failure's cause must reach the backup token; exemplar backup.rs count_store_text"
    );

    // A store closed under the backup reports that, not `Other`.
    g.store.close();
    let outcome = g.snap("", BACKUP_KEEP_DEFAULT);
    let failure = outcome.failure.expect("a closed store fails the backup");
    assert_eq!(
        (failure.phase, failure.kind),
        ("inventory", ErrorKind::BrokenPipe)
    );
    g.close();
}

#[test]
fn a_cancelled_capture_reads_nothing() {
    let root = scratch("backup-cancel");
    let graph = root.join("graph");
    for dir in ["pages", "journals", "assets", "logseq"] {
        std::fs::create_dir_all(graph.join(dir)).unwrap();
    }
    std::fs::write(graph.join("pages/note.md"), b"secret").unwrap();
    let (store, _, _) = Store::open(&graph, tine_store::OpenOptions::default()).unwrap();
    let mut out = Vec::new();
    let failure = read_store_area(
        &store,
        Area::Pages,
        "",
        is_graph_text,
        &|| true,
        &mut Budget::archive(),
        &mut out,
    )
    .unwrap_err();
    assert_eq!(failure.kind, ErrorKind::Interrupted);
    assert!(out.is_empty());
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn only_complete_v2_manifests_are_readable() {
    let root = scratch("backup-manifest");
    let manifest = SnapshotManifest {
        schema: LEGACY_SNAPSHOT_SCHEMA,
        root: root.display().to_string(),
        journals_dir: "diary".into(),
        pages_dir: "archive/pages".into(),
        graph_text_policy: None,
        writer: None,
        files: Vec::new(),
        complete: true,
    };
    write_manifest(&root, &manifest).unwrap();
    let read = read_manifest(&root).unwrap();
    assert_eq!(read.pages_dir, "archive/pages");
    assert!(verify_snapshot(&root, &read));
    std::fs::write(root.join("journals.md"), "- changed\n").unwrap();
    assert!(!verify_snapshot(&root, &read));
    std::fs::remove_file(root.join("journals.md")).unwrap();
    std::fs::write(
        root.join(SNAPSHOT_MANIFEST),
        r#"{"schema":2,"root":"x","journals_dir":"journals","pages_dir":"pages","files":[],"complete":false}"#,
    )
    .unwrap();
    assert!(read_manifest(&root).is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn asset_sidecars_are_captured_without_media_or_restore_recovery() {
    let g = Graph::new("capture-sidecars");
    let src = g.graph.join("assets");
    write(&src.join("doc.edn"), "{:a 1}\n");
    write(&src.join("nested/hl.edn"), "{:b 2}\n");
    write(&src.join("image.png"), "png");
    write(&src.join("nested/image.png"), "png");
    write(
        &src.join(ASSET_RESTORE_RECOVERY_DIR).join("old.edn"),
        "{:old true}\n",
    );
    let mut out = Vec::new();
    let read = read_store_area(
        &g.store,
        Area::Assets,
        "assets/",
        is_asset_sidecar,
        &|| false,
        &mut Budget::archive(),
        &mut out,
    );
    assert_eq!(read.unwrap(), 2);
    out.sort();
    assert_eq!(
        out,
        [
            ("assets/doc.edn".to_owned(), b"{:a 1}\n".to_vec()),
            ("assets/nested/hl.edn".to_owned(), b"{:b 2}\n".to_vec()),
        ]
    );
    g.close();
}

mod fail_read_tests {
    use super::*;
    #[test]
    fn fail_read_backup_refusal_keeps_phase_and_io_kind() {
        let error = rewrite_snapshot_result(BackupOutcome::failed(
            0,
            "pages",
            ErrorKind::PermissionDenied,
        ))
        .unwrap_err();
        assert_eq!(error, "backup-failed:pages:PermissionDenied");
    }
}

mod launch_schedule_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn launch_backup_does_not_copy_while_startup_is_still_running() {
        let root = tempfile::tempdir().unwrap();
        let slot = Arc::new(GraphSlot::new(
            Store::open(root.path(), Default::default()).unwrap().0,
            root.path().to_path_buf(),
        ));
        let (sent, received) = std::sync::mpsc::channel();
        let worker_slot = slot.clone();
        let worker = std::thread::spawn(move || {
            sent.send(wait_launch_backup(&worker_slot)).unwrap();
        });
        let early = received.recv_timeout(Duration::from_millis(1200));
        // End the worker after observing the result, even on the old schedule.
        slot.cancel_background();
        worker.join().unwrap();
        assert!(early.is_err(), "I-20: launch backup must wait for the owning warm completion signal; exemplar src-tauri/src/backup.rs");
    }
}

mod idle_signal_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn warm_completion_quiet_deadline_and_revocation_own_the_backup_wait() {
        let root = tempfile::tempdir().unwrap();
        let slot = Arc::new(GraphSlot::new(
            Store::open(root.path(), Default::default()).unwrap().0,
            root.path().to_path_buf(),
        ));
        let old = slot.begin_startup_warm();
        let current = slot.begin_startup_warm();
        slot.finish_startup_warm(old);
        assert!(
            !slot.warm_done.load(Ordering::Acquire),
            "an old warm cannot release the current backup"
        );
        let (sent, received) = std::sync::mpsc::channel();
        let waiting = slot.clone();
        let worker = std::thread::spawn(move || {
            sent.send(waiting.wait_startup_idle(Duration::from_millis(80), Duration::from_secs(5)))
                .unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(20)).is_err());
        slot.finish_startup_warm(current);
        assert!(
            received.recv_timeout(Duration::from_millis(20)).is_err(),
            "completion must retain a quiet turn"
        );
        assert!(received.recv_timeout(Duration::from_secs(2)).unwrap());
        worker.join().unwrap();
        // A missed warm signal still preserves the safety net at the deadline.
        slot.begin_startup_warm();
        assert!(slot.wait_startup_idle(Duration::from_secs(5), Duration::from_millis(1)));
        let waiting = slot.clone();
        let worker = std::thread::spawn(move || {
            waiting.wait_startup_idle(Duration::from_secs(5), Duration::from_secs(180))
        });
        slot.cancel_background();
        assert!(!worker.join().unwrap());
    }
}
