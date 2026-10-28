//! Autocommit against real repositories (ADR 0078 slice 1, review
//! `tine-agents/subagent-tasks/notes/review-adr-0078-git.md`).

use super::*;
use git2::{Repository, Signature};
use std::fs;

#[cfg(unix)]
fn directory_alias(target: &Path, path: &Path) {
    std::os::unix::fs::symlink(target, path).unwrap();
}

#[cfg(windows)]
fn directory_alias(target: &Path, path: &Path) {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    // libgit2 returns forward-slash paths; mklink treats those as switches.
    // Preserve UTF-16 and verbatim prefixes while changing only separators.
    let native = |path: &Path| {
        std::ffi::OsString::from_wide(
            &path
                .as_os_str()
                .encode_wide()
                .map(|c| if c == b'/' as u16 { b'\\' as u16 } else { c })
                .collect::<Vec<_>>(),
        )
    };
    let output = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(native(path))
        .arg(native(target))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mklink /J must create the Git containment fixture: {:?}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(same_file::is_same_file(target, path).unwrap());
}

#[test]
fn sb1_autocommit_refuses_root_replacement_before_index_lock() {
    let f = fixture(&[("pages/a.md", "- old\n")]);
    fs::write(f.work.join("pages/a.md"), "- pending\n").unwrap();
    let root = f.work.clone();
    let moved = f._dir.path().join("moved");
    super::super::git_run::lock_test_hook::install(move || {
        fs::rename(&root, &moved).unwrap();
        fs::create_dir(&root).unwrap();
        Repository::init(&root).unwrap();
    });
    let error = commit_now(&f.work, "autosave").unwrap_err();
    assert!(error.message().contains("GraphRootMoved"), "{error:?}");
    assert!(!f.work.join(".git/index.lock").exists());
    assert!(!f.work.join(".git/index").exists());
}

/// s3.7 P1/P1b: when the branch moves, the index already durably stages the
/// commit's blob (installed, and `.git` synced after the rename). Moving the
/// ref first, or leaving the index rename unsynced, lets a crash or power cut
/// bring the old index back under the new `HEAD`, and the user's next plain
/// `git commit` would record a reversion of the autocommit.
#[test]
fn autocommit_index_is_durable_before_the_branch_moves() {
    use super::super::git_run::lock_test_hook;
    let f = fixture(&[("pages/a.md", "- old\n")]);
    fs::write(f.work.join("pages/a.md"), "- new\n").unwrap();
    let gitdir = f.repo.path().to_path_buf();
    let seen = std::rc::Rc::new(std::cell::Cell::new(None));
    let record = seen.clone();
    let _ = tine_store::directory_durability::take_synced_directories();
    lock_test_hook::install_before_ref_move(move || {
        record.set(Some(lock_test_hook::durable_entry(&gitdir, "pages/a.md")));
    });
    assert!(matches!(
        commit_now(&f.work, "autosave"),
        Ok(Outcome::Committed(_))
    ));
    let (staged, synced) = seen.take().expect("the branch moved");
    let blob = git2::Oid::hash_object(git2::ObjectType::Blob, b"- new\n").unwrap();
    assert_eq!(
        staged,
        Some(blob),
        "the index is installed before the ref move"
    );
    // Unix: a real `.git` fsync. Windows: the graph folders' entry sync, with
    // identity checks and no directory flush (D-20261028-WDD, ADR 0078 §2);
    // the census row for `IndexLock::install` pins `sync_directory_witness`,
    // never the private (flushing) variant.
    #[cfg(unix)]
    assert!(synced, "`.git` is fsynced before the ref move");
    #[cfg(windows)]
    assert!(
        synced,
        "`.git`'s entry sync (no directory flush on Windows) runs before the ref move"
    );
}

#[cfg(unix)]
#[test]
fn sb1_index_lock_refuses_a_redirected_index() {
    let f = fixture(&[("pages/a.md", "- old\n")]);
    let foreign = f._dir.path().join("foreign-index");
    let original = fs::read(f.repo.path().join("index")).unwrap();
    fs::write(&foreign, &original).unwrap();
    fs::remove_file(f.repo.path().join("index")).unwrap();
    std::os::unix::fs::symlink(&foreign, f.repo.path().join("index")).unwrap();
    fs::write(f.work.join("pages/a.md"), "- pending\n").unwrap();
    assert!(commit_now(&f.work, "autosave").is_err());
    assert!(fs::symlink_metadata(f.repo.path().join("index"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(foreign).unwrap(), original);
}

#[test]
fn sb1_index_cleanup_preserves_files_after_metadata_root_replacement() {
    let f = fixture(&[("pages/a.md", "- old\n")]);
    let checked = open_checked(&f.work).unwrap();
    let lock = IndexLock::acquire(&f.repo, checked.guard).unwrap();
    let gitdir = f.work.join(".git");
    drop(checked.repo);
    drop(f.repo);
    let names: Vec<_> = fs::read_dir(&gitdir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().starts_with("tine-index-") || name == "index.lock")
        .collect();
    assert_eq!(names.len(), 2, "the held lock and private index must exist");
    // Windows refuses to rename an ancestor of an open file. Move the held
    // lock (std opens it with delete sharing) outside first, then put it back
    // in the detached metadata. Keep IndexLock alive so its real Drop runs
    // against the replacement, with every preservation assertion intact.
    let held = f._dir.path().join("held-index.lock");
    fs::rename(gitdir.join("index.lock"), &held).unwrap();
    let moved = f._dir.path().join("old-gitdir");
    fs::rename(&gitdir, &moved).unwrap();
    fs::rename(&held, moved.join("index.lock")).unwrap();
    fs::create_dir(&gitdir).unwrap();
    for name in &names {
        fs::write(gitdir.join(name), b"replacement-owned").unwrap();
    }
    drop(lock);
    for name in &names {
        assert_eq!(fs::read(gitdir.join(name)).unwrap(), b"replacement-owned");
        assert!(
            moved.join(name).exists(),
            "detached lock/copy left for inspection"
        );
    }
}

#[cfg(any(unix, windows))]
#[test]
fn sb1_repack_refuses_redirected_pack_directory() {
    let f = fixture(&[("pages/a.md", "- old\n")]);
    let outside = f._dir.path().join("foreign-pack");
    fs::create_dir(&outside).unwrap();
    let pack = f.repo.path().join("objects/pack");
    fs::rename(&pack, f._dir.path().join("old-pack")).unwrap();
    directory_alias(&outside, &pack);
    assert!(repack(&f.repo).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[cfg(any(unix, windows))]
#[test]
fn sb1_libgit_refuses_redirected_object_fanout() {
    let f = fixture(&[("pages/a.md", "- old\n")]);
    let outside = f._dir.path().join("foreign-objects");
    fs::create_dir(&outside).unwrap();
    let objects = f.repo.path().join("objects");
    let slot = (0..=255u8)
        .map(|n| objects.join(format!("{n:02x}")))
        .find(|path| !path.exists())
        .unwrap();
    directory_alias(&outside, &slot);
    fs::write(f.work.join("pages/a.md"), "- pending\n").unwrap();
    assert!(commit_now(&f.work, "autosave").is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[cfg(any(unix, windows))]
#[test]
fn sb1_git_directory_alias_is_refused_before_any_metadata_write() {
    let f = fixture(&[("pages/a.md", "- old\n")]);
    let gitdir = f.work.join(".git");
    let outside = f._dir.path().join("redirected-gitdir");
    fs::rename(&gitdir, &outside).unwrap();
    directory_alias(&outside, &gitdir);
    let index = fs::read(outside.join("index")).unwrap();
    let head = fs::read(outside.join("HEAD")).unwrap();
    fs::write(f.work.join("pages/a.md"), "- pending\n").unwrap();
    assert!(commit_now(&f.work, "autosave").is_err());
    assert_eq!(fs::read(outside.join("index")).unwrap(), index);
    assert_eq!(fs::read(outside.join("HEAD")).unwrap(), head);
    assert!(!outside.join("index.lock").exists());
}

struct Fixture {
    _dir: tempfile::TempDir,
    work: PathBuf,
    repo: Repository,
}

#[test]
fn sb1_linked_worktree_keeps_its_selected_git_metadata_scope() {
    let f = fixture(&[("pages/a.md", "- old\n")]);
    let tip = head(&f.repo).id();
    let linked = f._dir.path().join("linked");
    f.repo.worktree("linked", &linked, None).unwrap();
    assert!(linked.join(".git").is_file());
    fs::write(linked.join("pages/a.md"), "- linked edit\n").unwrap();
    assert!(matches!(
        commit_now(&linked, "autosave"),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(head(&f.repo).id(), tip);
    assert_eq!(
        at_head(&Repository::open(linked).unwrap(), "pages/a.md").as_deref(),
        Some("- linked edit\n")
    );
}

/// A repository at `work` holding `files`, all committed by the user.
fn fixture(files: &[(&str, &str)]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    fs::create_dir_all(&work).unwrap();
    let repo = Repository::init(&work).unwrap();
    {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Martin").unwrap();
        config.set_str("user.email", "martin@example.org").unwrap();
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        config.set_bool("core.precomposeunicode", true).unwrap();
    }
    for (path, text) in files {
        write(&work, path, text);
    }
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let me = Signature::now("Martin", "martin@example.org").unwrap();
    repo.commit(Some("HEAD"), &me, &me, "initial", &tree, &[])
        .unwrap();
    drop(tree);
    Fixture {
        _dir: dir,
        work,
        repo,
    }
}

fn write(work: &Path, path: &str, text: &str) {
    let full = work.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
}

fn head(repo: &Repository) -> git2::Commit<'_> {
    repo.head().unwrap().peel_to_commit().unwrap()
}

fn at_head(repo: &Repository, path: &str) -> Option<String> {
    let entry = head(repo).tree().unwrap().get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8(blob.content().to_vec()).unwrap())
}

/// `git status` paths that differ, as libgit2 sees them (ignored excluded).
fn dirty(repo: &Repository) -> Vec<String> {
    let mut options = StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    let mut out: Vec<String> = repo
        .statuses(Some(&mut options))
        .unwrap()
        .iter()
        .map(|e| e.path().unwrap().to_string())
        .collect();
    out.sort();
    out
}

#[test]
fn an_autocommit_records_every_change_under_the_root_and_leaves_the_tree_clean() {
    let f = fixture(&[("pages/a.md", "- a\n"), ("pages/b.md", "- b\n")]);
    write(&f.work, "pages/a.md", "- a changed\n");
    fs::remove_file(f.work.join("pages/b.md")).unwrap();
    write(&f.work, "pages/c.md", "- new by another editor\n");
    write(&f.work, "assets/x.png", "png");

    let outcome = commit_now(&f.work, MESSAGE).unwrap();
    let Outcome::Committed(oid) = outcome else {
        panic!("{outcome:?}")
    };
    let commit = head(&f.repo);
    assert_eq!(commit.id(), oid);
    assert_eq!(commit.message().ok(), Some(MESSAGE));
    assert_eq!(commit.author().email().ok(), Some("martin@example.org"));
    assert_eq!(commit.parent_count(), 1);
    assert_eq!(
        at_head(&f.repo, "pages/a.md").as_deref(),
        Some("- a changed\n")
    );
    assert_eq!(at_head(&f.repo, "pages/b.md"), None);
    assert_eq!(
        at_head(&f.repo, "pages/c.md").as_deref(),
        Some("- new by another editor\n")
    );
    assert_eq!(at_head(&f.repo, "assets/x.png").as_deref(), Some("png"));
    assert_eq!(
        dirty(&f.repo),
        Vec::<String>::new(),
        "index and work tree match HEAD"
    );
    assert!(!f.repo.path().join("index.lock").exists());

    assert_eq!(commit_now(&f.work, MESSAGE).unwrap(), Outcome::Clean);
    assert_eq!(head(&f.repo).id(), oid, "nothing to commit makes no commit");
}

#[test]
fn syncthing_temps_are_never_committed_but_conflict_copies_are() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    write(&f.work, "pages/.syncthing.a.md.tmp", "half a page");
    write(&f.work, ".syncthing.asset.tmp", "half an asset");
    write(
        &f.work,
        "pages/a.sync-conflict-20261026-device.md",
        "- other edits\n",
    );
    // Even a temp staged by another git program must not enter the commit.
    let mut index = f.repo.index().unwrap();
    index.add_path(Path::new(".syncthing.asset.tmp")).unwrap();
    index.write().unwrap();
    commit_now(&f.work, MESSAGE).unwrap();
    assert_eq!(at_head(&f.repo, "pages/.syncthing.a.md.tmp"), None);
    assert_eq!(at_head(&f.repo, ".syncthing.asset.tmp"), None);
    assert_eq!(
        at_head(&f.repo, "pages/a.sync-conflict-20261026-device.md").as_deref(),
        Some("- other edits\n")
    );
}

#[test]
fn syncthing_filter_is_relative_to_the_graph_and_retires_old_tracked_temps() {
    let f = fixture(&[
        (".syncthing.graph/pages/a.md", "- original\n"),
        (".syncthing.graph/pages/.syncthing.old.tmp", "old partial"),
        (".syncthing.outside.tmp", "outside graph"),
    ]);
    let root = f.work.join(".syncthing.graph");
    write(&root, "pages/a.md", "- edited\n");
    commit_now(&root, MESSAGE).unwrap();
    assert_eq!(
        at_head(&f.repo, ".syncthing.graph/pages/a.md").as_deref(),
        Some("- edited\n")
    );
    assert_eq!(
        at_head(&f.repo, ".syncthing.graph/pages/.syncthing.old.tmp"),
        None
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/.syncthing.old.tmp")).unwrap(),
        "old partial"
    );
    assert_eq!(
        at_head(&f.repo, ".syncthing.outside.tmp").as_deref(),
        Some("outside graph")
    );
}

#[test]
fn syncthing_ignore_is_appended_once_and_removal_survives_reopen() {
    for original in [
        None,
        Some("# keep\r\nassets/private"),
        Some("# keep\n"),
        Some(""),
    ] {
        let f = fixture(&[("pages/a.md", "- a\n")]);
        if let Some(text) = original {
            write(&f.work, ".gitignore", text);
        }
        let app_data = f._dir.path().join("device");
        let store = tine_store::Store::open(&f.work, Default::default())
            .unwrap()
            .0;
        write(&f.work, "pages/.syncthing.a.md.tmp", "partial");
        write(
            &f.work,
            "pages/a.sync-conflict-20261026-device.md",
            "- conflict\n",
        );
        assert!(prepare_syncthing_ignore(&store, &f.work, &app_data).unwrap());
        let newline = if original.is_some_and(|s| s.contains("\r\n")) {
            "\r\n"
        } else {
            "\n"
        };
        let mut expected = original.unwrap_or_default().to_owned();
        if !expected.is_empty() && !expected.ends_with('\n') {
            expected.push_str(newline);
        }
        expected.push_str(".syncthing.*");
        expected.push_str(newline);
        assert_eq!(
            fs::read_to_string(f.work.join(".gitignore")).unwrap(),
            expected
        );
        commit_now(&f.work, MESSAGE).unwrap();
        assert_eq!(at_head(&f.repo, ".gitignore"), Some(expected.clone()));
        assert_eq!(at_head(&f.repo, "pages/.syncthing.a.md.tmp"), None);
        assert_eq!(
            at_head(&f.repo, "pages/a.sync-conflict-20261026-device.md").as_deref(),
            Some("- conflict\n")
        );
        assert!(!prepare_syncthing_ignore(&store, &f.work, &app_data).unwrap());
        assert_eq!(
            fs::read_to_string(f.work.join(".gitignore")).unwrap(),
            expected
        );
        assert_eq!(commit_now(&f.work, MESSAGE).unwrap(), Outcome::Clean);

        write(&f.work, ".gitignore", "# user removed the rule\n");
        drop(store);
        let reopened = tine_store::Store::open(&f.work, Default::default())
            .unwrap()
            .0;
        assert!(!prepare_syncthing_ignore(&reopened, &f.work, &app_data).unwrap());
        assert_eq!(
            fs::read_to_string(f.work.join(".gitignore")).unwrap(),
            "# user removed the rule\n"
        );
    }
}

#[test]
fn syncthing_ignore_uses_git_rules_and_only_writes_the_nested_graph() {
    for rule in [Some(".syncthing.*\n"), Some(".*\n"), None] {
        let f = fixture(&[("Notes [work]/pages/a.md", "- a\n")]);
        let root = f.work.join("Notes [work]");
        let original = rule.unwrap_or("# parent stays untouched\n");
        write(&f.work, ".gitignore", original);
        let store = tine_store::Store::open(&root, Default::default())
            .unwrap()
            .0;
        let app_data = f._dir.path().join("device");
        assert_eq!(
            prepare_syncthing_ignore(&store, &root, &app_data).unwrap(),
            rule.is_none()
        );
        assert_eq!(
            fs::read_to_string(f.work.join(".gitignore")).unwrap(),
            original
        );
        if rule.is_none() {
            assert_eq!(
                fs::read_to_string(root.join(".gitignore")).unwrap(),
                ".syncthing.*\n"
            );
        } else {
            assert!(
                !root.join(".gitignore").exists(),
                "parent pattern already covers temps"
            );
            write(&f.work, ".gitignore", "# removed broader user rule\n");
            assert!(!prepare_syncthing_ignore(&store, &root, &app_data).unwrap());
            assert!(!root.join(".gitignore").exists());
        }
    }
}

#[test]
fn syncthing_ignore_failure_never_allows_a_temp_commit_or_retries_the_write() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let store = tine_store::Store::open(&f.work, Default::default())
        .unwrap()
        .0;
    let app_data = f._dir.path().join("device");
    fs::create_dir(f.work.join(".gitignore")).unwrap();
    assert!(prepare_syncthing_ignore(&store, &f.work, &app_data).is_err());
    write(&f.work, "pages/.syncthing.a.md.tmp", "partial");
    write(
        &f.work,
        "pages/a.sync-conflict-20261026-device.md",
        "- conflict\n",
    );
    commit_now(&f.work, MESSAGE).unwrap();
    assert_eq!(at_head(&f.repo, "pages/.syncthing.a.md.tmp"), None);
    assert_eq!(
        at_head(&f.repo, "pages/a.sync-conflict-20261026-device.md").as_deref(),
        Some("- conflict\n")
    );
    fs::remove_dir(f.work.join(".gitignore")).unwrap();
    assert!(!prepare_syncthing_ignore(&store, &f.work, &app_data).unwrap());
    assert!(!f.work.join(".gitignore").exists());
}

#[test]
fn syncthing_ignore_does_not_write_without_durable_attempt_memory() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let store = tine_store::Store::open(&f.work, Default::default())
        .unwrap()
        .0;
    let app_data = f._dir.path().join("not-a-directory");
    fs::write(&app_data, "occupied").unwrap();
    assert!(prepare_syncthing_ignore(&store, &f.work, &app_data).is_err());
    assert!(!f.work.join(".gitignore").exists());
}

#[test]
fn root_gitignore_keeps_the_non_page_revision_and_target_guards() {
    use tine_store::{Content, FileId, TxOutcome, Why};
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let store = tine_store::Store::open(&f.work, Default::default())
        .unwrap()
        .0;
    let id = FileId::from(".gitignore".to_owned());
    assert!(store.as_page(&id).is_none());
    let mut tx = store.transaction(None);
    tx.create(&id, Content::Bytes(b"# original\n".to_vec()));
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    let (_, base) = store.read(&id, None).unwrap();
    write(&f.work, ".gitignore", "# outside edit\n");
    let mut tx = store.transaction(None);
    tx.replace(&id, base, b"# stale append\n.syncthing.*\n".to_vec());
    assert!(matches!(
        tx.commit(),
        TxOutcome::NotCommitted {
            why: Why::Conflict { .. },
            ..
        }
    ));
    assert_eq!(
        fs::read_to_string(f.work.join(".gitignore")).unwrap(),
        "# outside edit\n"
    );
    let mut tx = store.transaction(None);
    tx.create(&id, Content::Bytes(b"# overwrite\n".to_vec()));
    assert!(matches!(tx.commit(), TxOutcome::NotCommitted { .. }));
    for name in [
        ".git/config",
        ".other-root-file",
        "../.gitignore",
        "sub/.gitignore",
    ] {
        assert!(store.read(&FileId::from(name.to_owned()), None).is_err());
    }
}

#[cfg(unix)]
#[test]
fn syncthing_ignore_refuses_a_root_symlink_to_an_outside_file() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let outside = f._dir.path().join("outside-ignore");
    fs::write(&outside, "# outside\n").unwrap();
    std::os::unix::fs::symlink(&outside, f.work.join(".gitignore")).unwrap();
    let store = tine_store::Store::open(&f.work, Default::default())
        .unwrap()
        .0;
    assert!(prepare_syncthing_ignore(&store, &f.work, &f._dir.path().join("device")).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "# outside\n");
}

#[test]
fn ignored_files_temps_and_nested_repositories_are_left_out() {
    let f = fixture(&[(".gitignore", "logseq/bak/\n"), ("pages/a.md", "- a\n")]);
    write(&f.work, "logseq/bak/pages/a.md", "backup");
    write(&f.work, "pages/.a.md.4242.17.tmp", "half a save");
    write(&f.work, "pages/.a.md.4242.18new.tmp", "half a save");
    let nested = f.work.join("vendor/other");
    fs::create_dir_all(&nested).unwrap();
    Repository::init(&nested).unwrap();
    write(&nested, "x.md", "x");
    write(&f.work, "pages/a.md", "- a2\n");

    commit_now(&f.work, MESSAGE).unwrap();
    let tree = head(&f.repo).tree().unwrap();
    assert!(tree.get_path(Path::new("logseq/bak")).is_err());
    assert!(tree.get_path(Path::new("pages/.a.md.4242.17.tmp")).is_err());
    assert!(tree
        .get_path(Path::new("pages/.a.md.4242.18new.tmp"))
        .is_err());
    assert!(
        tree.get_path(Path::new("vendor")).is_err(),
        "no gitlink for a nested repository"
    );
    assert_eq!(at_head(&f.repo, "pages/a.md").as_deref(), Some("- a2\n"));
}

#[test]
fn a_graph_inside_a_bigger_repository_commits_only_itself_even_with_glob_characters() {
    let f = fixture(&[
        ("Notes [work]/pages/a.md", "- a\n"),
        ("Notes w/pages/a.md", "- sibling\n"),
        ("src/main.rs", "fn main() {}\n"),
    ]);
    let root = f.work.join("Notes [work]");
    write(&f.work, "Notes [work]/pages/a.md", "- a2\n");
    write(&f.work, "Notes w/pages/a.md", "- sibling changed\n");
    write(&f.work, "src/main.rs", "fn main() { staged }\n");
    // The user staged a change outside the graph.
    let mut index = f.repo.index().unwrap();
    index.add_path(Path::new("src/main.rs")).unwrap();
    index.write().unwrap();
    let staged = index.get_path(Path::new("src/main.rs"), 0).unwrap().id;

    commit_now(&root, MESSAGE).unwrap();
    assert_eq!(
        at_head(&f.repo, "Notes [work]/pages/a.md").as_deref(),
        Some("- a2\n")
    );
    assert_eq!(
        at_head(&f.repo, "Notes w/pages/a.md").as_deref(),
        Some("- sibling\n")
    );
    assert_eq!(
        at_head(&f.repo, "src/main.rs").as_deref(),
        Some("fn main() {}\n")
    );
    let index = f.repo.index().unwrap();
    assert_eq!(
        index.get_path(Path::new("src/main.rs"), 0).unwrap().id,
        staged,
        "the user's staged change outside the graph stays staged"
    );
    assert_eq!(
        dirty(&f.repo),
        vec!["Notes w/pages/a.md".to_string(), "src/main.rs".to_string()]
    );
}

#[test]
fn a_held_index_lock_refuses_and_changes_nothing() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    write(&f.work, "pages/a.md", "- a2\n");
    let lock = f.repo.path().join("index.lock");
    fs::write(&lock, "someone else's").unwrap();
    let before = head(&f.repo).id();
    assert_eq!(commit_now(&f.work, MESSAGE), Err(Unusable::Busy));
    assert_eq!(head(&f.repo).id(), before);
    assert_eq!(
        fs::read_to_string(&lock).unwrap(),
        "someone else's",
        "never touched"
    );
}

#[test]
fn a_content_filter_refuses() {
    let f = fixture(&[
        (".gitattributes", "*.md filter=crypt\n"),
        ("pages/a.md", "- a\n"),
    ]);
    write(&f.work, "pages/a.md", "- secret\n");
    let refused = commit_now(&f.work, MESSAGE).unwrap_err();
    assert_eq!(refused.code(), "conversion", "{refused:?}");
    assert_eq!(at_head(&f.repo, "pages/a.md").as_deref(), Some("- a\n"));
}

#[test]
fn eol_normalisation_does_not_refuse_a_commit() {
    let f = fixture(&[(".gitattributes", "* text=auto\n"), ("pages/a.md", "- a\n")]);
    f.repo
        .config()
        .unwrap()
        .set_str("core.autocrlf", "input")
        .unwrap();
    write(&f.work, "pages/a.md", "- a2\n");
    assert!(matches!(
        commit_now(&f.work, MESSAGE),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(at_head(&f.repo, "pages/a.md").as_deref(), Some("- a2\n"));
}

/// Review R7: libgit2 reports a skip-worktree file missing from the work
/// tree as deleted; committing that would delete it on every device.
#[test]
fn skip_worktree_entries_refuse_instead_of_committing_deletions() {
    let f = fixture(&[("pages/a.md", "- a\n"), ("pages/b.md", "- b\n")]);
    let mut index = f.repo.index().unwrap();
    let mut entry = index.get_path(Path::new("pages/b.md"), 0).unwrap();
    entry.flags |= 0x4000; // extended
    entry.flags_extended |= 1 << 14; // skip-worktree
    index.add(&entry).unwrap();
    index.write().unwrap();
    fs::remove_file(f.work.join("pages/b.md")).unwrap();
    write(&f.work, "pages/a.md", "- a2\n");
    assert_eq!(commit_now(&f.work, MESSAGE), Err(Unusable::Sparse));
    assert_eq!(at_head(&f.repo, "pages/b.md").as_deref(), Some("- b\n"));

    f.repo
        .config()
        .unwrap()
        .set_bool("core.sparseCheckout", true)
        .unwrap();
    assert_eq!(commit_now(&f.work, MESSAGE), Err(Unusable::Sparse));
}

/// G5: every change git sees. A file the user untracked but did not ignore
/// is still there for git, so it stays committed (review F2: skipping it
/// only deleted it once, and made a lost index a mass deletion).
#[test]
fn a_file_untracked_but_not_ignored_stays_committed() {
    let f = fixture(&[("pages/a.md", "- a\n"), ("pages/private.md", "- p\n")]);
    let mut index = f.repo.index().unwrap();
    index.remove_path(Path::new("pages/private.md")).unwrap();
    index.write().unwrap();
    write(&f.work, "pages/a.md", "- a2\n");
    commit_now(&f.work, MESSAGE).unwrap();
    assert_eq!(
        at_head(&f.repo, "pages/private.md").as_deref(),
        Some("- p\n")
    );
    assert!(dirty(&f.repo).is_empty(), "{:?}", dirty(&f.repo));
}

/// Review B1: committing from a missing index would delete every file.
#[test]
fn a_missing_index_refuses_instead_of_committing_deletions() {
    let f = fixture(&[("pages/a.md", "- a\n"), ("pages/b.md", "- b\n")]);
    write(&f.work, "pages/a.md", "- a2\n");
    fs::remove_file(f.repo.path().join("index")).unwrap();
    let before = head(&f.repo).id();
    let refused = commit_now(&f.work, MESSAGE).unwrap_err();
    assert_eq!(refused.code(), "repository", "{refused:?}");
    assert!(
        refused.message().contains("git reset"),
        "{}",
        refused.message()
    );
    assert_eq!(head(&f.repo).id(), before);
    assert!(!f.repo.path().join("index").exists(), "nothing installed");
    assert!(!f.repo.path().join("index.lock").exists(), "lock released");
}

/// Review B1(b): another holder's live copy is never removed: copies are
/// cleaned only by a holder of `index.lock`.
#[test]
fn a_live_index_copy_survives_while_another_holds_the_lock() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let held = IndexLock::acquire(&f.repo, open_checked(&f.work).unwrap().guard).unwrap();
    let copies = |dir: &Path| {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("tine-index-"))
            .count()
    };
    assert_eq!(copies(f.repo.path()), 1);
    write(&f.work, "pages/a.md", "- a2\n");
    assert_eq!(commit_now(&f.work, MESSAGE), Err(Unusable::Busy));
    assert_eq!(copies(f.repo.path()), 1, "the holder's copy is untouched");
    drop(held);
    assert_eq!(copies(f.repo.path()), 0);
}

/// Review B2: `.gitattributes` applies by pattern, so every staged file is
/// checked, not only sample paths.
#[test]
fn an_lfs_tracked_asset_refuses_the_commit() {
    let f = fixture(&[
        (
            ".gitattributes",
            "*.pdf filter=lfs diff=lfs merge=lfs -text\n",
        ),
        ("pages/a.md", "- a\n"),
    ]);
    assert!(git_status(&f.work, GitSettings::default())
        .unusable
        .is_none());
    write(&f.work, "pages/a.md", "- a2\n");
    write(&f.work, "assets/paper.pdf", "%PDF raw");
    let refused = commit_now(&f.work, MESSAGE).unwrap_err();
    assert_eq!(refused.code(), "conversion", "{refused:?}");
    assert!(
        refused.message().contains("assets/paper.pdf"),
        "{}",
        refused.message()
    );
    assert_eq!(at_head(&f.repo, "assets/paper.pdf"), None);
    assert_eq!(at_head(&f.repo, "pages/a.md").as_deref(), Some("- a\n"));
}

#[test]
fn a_conflicted_index_is_named_not_busy() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let mut index = f.repo.index().unwrap();
    let mut entry = index.get_path(Path::new("pages/a.md"), 0).unwrap();
    index.remove_path(Path::new("pages/a.md")).unwrap();
    for stage in [1u16, 2, 3] {
        entry.flags = (entry.flags & !0x3000) | (stage << 12);
        index.add(&entry).unwrap();
    }
    index.write().unwrap();
    assert_eq!(commit_now(&f.work, MESSAGE), Err(Unusable::Conflicted));
}

#[test]
fn not_a_repository_and_a_detached_head_are_named() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        commit_now(dir.path(), MESSAGE),
        Err(Unusable::NotARepository)
    );
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let id = head(&f.repo).id();
    f.repo.set_head_detached(id).unwrap();
    assert_eq!(
        commit_now(&f.work, MESSAGE).unwrap_err().code(),
        "repository"
    );
}

#[test]
fn a_first_commit_on_an_unborn_branch_works() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Martin").unwrap();
        config.set_str("user.email", "martin@example.org").unwrap();
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        config.set_bool("core.precomposeunicode", true).unwrap();
    }
    write(dir.path(), "pages/a.md", "- a\n");
    assert!(matches!(
        commit_now(dir.path(), MESSAGE),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(head(&repo).parent_count(), 0);
    assert_eq!(at_head(&repo, "pages/a.md").as_deref(), Some("- a\n"));
}

/// Review R4: only loose objects the new pack holds are deleted.
#[test]
fn repack_deletes_only_packed_loose_objects() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    for n in 0..5 {
        write(&f.work, "pages/a.md", &format!("- a{n}\n"));
        commit_now(&f.work, MESSAGE).unwrap();
    }
    // Referenced by nothing but the user's index: must survive.
    let staged_only = f.repo.blob(b"staged only").unwrap();
    let objects = f.repo.path().join("objects");
    let before = loose_object_count(&objects);
    let removed = repack(&f.repo).unwrap();
    assert_eq!(
        removed,
        Some(before),
        "every loose object is packed, then deleted"
    );
    // Every object, reachable or not, is still readable.
    let reopened = Repository::open(&f.work).unwrap();
    assert_eq!(at_head(&reopened, "pages/a.md").as_deref(), Some("- a4\n"));
    assert_eq!(
        reopened.find_blob(staged_only).unwrap().content(),
        b"staged only",
        "an unreachable object is packed, never dropped"
    );
    let mut walk = reopened.revwalk().unwrap();
    walk.push_head().unwrap();
    assert_eq!(walk.count(), 6);
    // A second repack with nothing new loose deletes nothing more.
    assert_eq!(repack(&reopened).unwrap(), Some(0));
}

/// Review N1: a repack packs only what was added since the last one; the
/// packed history is never rewritten.
#[test]
fn a_repack_packs_only_new_objects() {
    let f = fixture(&[("pages/a.md", "- a\n"), ("pages/b.md", "- b\n")]);
    let pack_dir = f.repo.path().join("objects/pack");
    let packs = || -> Vec<(PathBuf, u64)> {
        let mut out: Vec<_> = fs::read_dir(&pack_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "pack"))
            .map(|e| (e.path(), e.metadata().unwrap().len()))
            .collect();
        out.sort();
        out
    };
    for n in 0..20 {
        write(
            &f.work,
            "pages/a.md",
            &format!("- a{n}\n{}\n", "x".repeat(400)),
        );
        commit_now(&f.work, MESSAGE).unwrap();
    }
    repack(&f.repo).unwrap();
    let first = packs();
    assert_eq!(first.len(), 1);
    write(&f.work, "pages/b.md", "- b changed\n");
    commit_now(&f.work, MESSAGE).unwrap();
    let loose = loose_object_count(&f.repo.path().join("objects"));
    assert_eq!(repack(&f.repo).unwrap(), Some(loose));
    let second = packs();
    assert_eq!(second.len(), 2);
    assert!(second.contains(&first[0]), "the first pack is untouched");
    let added = second.iter().find(|p| !first.contains(p)).unwrap();
    assert!(added.1 < first[0].1, "{} vs {}", added.1, first[0].1);
}

#[test]
fn repack_waits_for_a_running_git_gc() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    fs::write(f.repo.path().join("gc.pid"), "1 host").unwrap();
    assert_eq!(repack(&f.repo).unwrap(), None);
    assert!(loose_object_count(&f.repo.path().join("objects")) > 0);
}

#[test]
fn failed_repack_preserves_loose_objects_and_commits_continue() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let objects = f.repo.path().join("objects");
    let before = loose_objects(&objects);
    // An interrupted/external writer left an unreadable loose object. Fail
    // packing through real libgit2 I/O, without depending on OS permissions.
    let broken = objects.join("ab").join("c".repeat(38));
    fs::create_dir_all(broken.parent().unwrap()).unwrap();
    fs::write(&broken, b"not a compressed git object").unwrap();
    let (worker, _) = registry::get_or_insert(&f.work);
    maybe_repack(&worker, &f.work, true);
    for oid in &before {
        let hex = oid.to_string();
        assert!(objects.join(&hex[..2]).join(&hex[2..]).is_file());
    }
    let last = git_status(&f.work, GitSettings::default()).last;
    assert!(
        last.refused.is_none(),
        "cleanup must not claim commits failed"
    );
    assert_eq!(serde_json::to_value(last).unwrap()["repack_deferred"], true);
    write(&f.work, "pages/a.md", "- saved while cleanup waits\n");
    assert!(matches!(
        commit_now(&f.work, MESSAGE),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(
        at_head(&f.repo, "pages/a.md").as_deref(),
        Some("- saved while cleanup waits\n")
    );
    fs::remove_file(broken).unwrap();
    // Even the loose-object-triggered failure (no 50-commit counter) must
    // remain pending. A successful commit must not cause immediate retry.
    assert_eq!(worker.state.lock().unwrap().commits_since_repack, 0);
    maybe_repack(&worker, &f.work, true);
    assert!(!loose_objects(&objects).is_empty(), "retry is delayed");
    {
        let mut state = worker.state.lock().unwrap();
        assert!(state.last.repack_deferred);
        state.repack_not_before = Some(Instant::now());
    }
    // Waiting for another git process is a deferral, not recovery.
    let gc_pid = f.repo.path().join("gc.pid");
    fs::write(&gc_pid, "1 host").unwrap();
    maybe_repack(&worker, &f.work, true);
    assert!(worker.state.lock().unwrap().last.repack_deferred);
    assert!(!loose_objects(&objects).is_empty());
    fs::remove_file(gc_pid).unwrap();
    worker.state.lock().unwrap().repack_not_before = Some(Instant::now());
    maybe_repack(&worker, &f.work, true);
    assert!(
        !git_status(&f.work, GitSettings::default())
            .last
            .repack_deferred
    );
    assert!(
        loose_objects(&objects).is_empty(),
        "a later cycle retries cleanup"
    );
    let reopened = Repository::open(&f.work).unwrap();
    assert_eq!(
        at_head(&reopened, "pages/a.md").as_deref(),
        Some("- saved while cleanup waits\n")
    );
    registry::unregister(&worker);
}

#[test]
fn around_brackets_an_operation_with_two_commits_when_the_graph_autocommits() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let (worker, _) = registry::get_or_insert(&f.work);
    worker.state.lock().unwrap().settings = GitSettings {
        policy: Policy::Commit,
        interval: minutes(10),
    };
    write(&f.work, "pages/a.md", "- pending\n");
    let before = head(&f.repo).id();
    let result: Result<(), String> = around(
        &f.work,
        || "Rename page \"a\" to \"b\"".into(),
        || {
            fs::rename(f.work.join("pages/a.md"), f.work.join("pages/b.md")).unwrap();
            Ok(())
        },
    );
    result.unwrap();
    let op = head(&f.repo);
    assert_eq!(op.message().ok(), Some("Rename page \"a\" to \"b\""));
    let snapshot = op.parent(0).unwrap();
    assert_eq!(snapshot.message().ok(), Some(MESSAGE));
    assert_eq!(snapshot.parent_id(0).unwrap(), before);
    assert_eq!(
        at_head(&f.repo, "pages/b.md").as_deref(),
        Some("- pending\n")
    );

    // A failed operation leaves only the first commit.
    write(&f.work, "pages/b.md", "- more\n");
    let failed: Result<(), String> = around(
        &f.work,
        || "Delete page \"b\"".into(),
        || Err("refused".into()),
    );
    assert!(failed.is_err());
    assert_eq!(head(&f.repo).message().ok(), Some(MESSAGE));
    registry::unregister(&worker);

    // Off: no commits at all.
    write(&f.work, "pages/b.md", "- off\n");
    let tip = head(&f.repo).id();
    let _: Result<(), String> = around(&f.work, || "x".into(), || Ok(()));
    assert_eq!(head(&f.repo).id(), tip);
}

#[test]
fn settings_round_trip_through_the_device_settings_json() {
    let root = Path::new("/graphs/Notes");
    let mut json = serde_json::json!({"backup_keep": 7});
    assert_eq!(settings_from_json(&json, root), GitSettings::default());
    settings_into_json(
        &mut json,
        root,
        GitSettings {
            policy: Policy::Commit,
            interval: minutes(25),
        },
    );
    assert_eq!(json["backup_keep"], 7);
    let read = settings_from_json(&json, root);
    assert_eq!((read.policy, read.interval), (Policy::Commit, minutes(25)));
    json[SETTINGS_KEY][settings_key(root)]["interval_minutes"] = serde_json::json!(0);
    assert_eq!(
        settings_from_json(&json, root).interval,
        minutes(1),
        "clamped"
    );
    json[SETTINGS_KEY][settings_key(root)]["policy"] = serde_json::json!("sync-everything");
    assert_eq!(
        settings_from_json(&json, root).policy,
        Policy::Off,
        "unknown reads as Off"
    );
}

#[test]
fn tine_temp_matches_only_the_atomic_save_shape() {
    assert!(tine_temp("pages/.a.md.123.4.tmp"));
    assert!(tine_temp(".config.edn.9.10new.tmp"));
    assert!(!tine_temp("pages/a.md"));
    assert!(!tine_temp("pages/notes.tmp"));
    assert!(!tine_temp("pages/.hidden.tmp"));
    assert!(!tine_temp("pages/a.x.y.tmp"));
}

#[test]
fn a_crash_left_index_copy_is_removed_only_under_the_lock() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    let gitdir = f.repo.path();
    fs::write(gitdir.join("tine-index-abc.tmp"), "x").unwrap();
    fs::write(gitdir.join("tine-index-abc.tmp.lock"), "x").unwrap();
    fs::write(gitdir.join("index.lock"), "x").unwrap();
    // Someone else's lock: nothing is cleaned.
    assert_eq!(commit_now(&f.work, MESSAGE), Err(Unusable::Busy));
    assert!(gitdir.join("tine-index-abc.tmp").exists());
    assert!(gitdir.join("index.lock").exists());
    // Once the lock is free, the next holder removes the leftovers.
    fs::remove_file(gitdir.join("index.lock")).unwrap();
    drop(IndexLock::acquire(&f.repo, open_checked(&f.work).unwrap().guard).unwrap());
    assert!(!gitdir.join("tine-index-abc.tmp").exists());
    assert!(!gitdir.join("tine-index-abc.tmp.lock").exists());
    assert!(!gitdir.join("index.lock").exists());
}

/// I-25 measurement (ADR 0078 Unit cost): on 10 000 flat pages, what one
/// incremental repack of 50 one-page autocommits adds to `.git`. Run with
/// `cargo test -p tine --lib -- --ignored measure_incremental_repack --nocapture`.
#[test]
#[ignore]
fn measure_incremental_repack() {
    let pages: Vec<(String, String)> = (0..10_000)
        .map(|n| {
            (
                format!("pages/page {n}.md"),
                format!("- page {n} {}\n", "x".repeat(500)),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = pages
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let f = fixture(&refs);
    let pack_bytes = |repo: &Repository| -> u64 {
        fs::read_dir(repo.path().join("objects/pack"))
            .map(|d| d.flatten().map(|e| e.metadata().unwrap().len()).sum())
            .unwrap_or(0)
    };
    repack(&f.repo).unwrap();
    let base = pack_bytes(&f.repo);
    for round in 0..2 {
        let started = Instant::now();
        for n in 0..50 {
            write(
                &f.work,
                &format!("pages/page {}.md", n * 37),
                &format!("- edit {round} {n}\n"),
            );
            commit_now(&f.work, MESSAGE).unwrap();
        }
        let loose = loose_object_count(&f.repo.path().join("objects"));
        let before = pack_bytes(&f.repo);
        let t = Instant::now();
        repack(&f.repo).unwrap();
        let added = pack_bytes(&f.repo) - before;
        println!(
            "round {round}: initial pack {base} B; 50 commits ({:?}) -> {loose} loose objects; repack {:?} adds {added} B ({} B per commit)",
            started.elapsed(),
            t.elapsed(),
            added / 50
        );
    }
}

/// ADR 0078 identity: an autocommit in a repository with no `user.name` /
/// `user.email` commits under the fallback (no "Your name" in unit tests),
/// with only a name under that name and the placeholder email; seen's "me"
/// recognizes exactly those commits.
#[test]
fn an_autocommit_without_a_repository_identity_uses_the_shared_identity_rule() {
    let f = fixture(&[("pages/a.md", "- a\n")]);
    {
        let mut config = f.repo.config().unwrap();
        // Empty local values hide any developer-global identity.
        config.set_str("user.name", "").unwrap();
        config.set_str("user.email", "").unwrap();
    }
    write(&f.work, "pages/a.md", "- a changed\n");
    commit_now(&f.work, MESSAGE).unwrap();
    let author = head(&f.repo).author().to_owned();
    assert_eq!(
        (author.name().ok(), author.email().ok()),
        (
            Some(super::super::git_run::USER_FALLBACK.0),
            Some(super::super::git_run::USER_FALLBACK.1)
        )
    );
    let me = super::super::git_run::user_identity(&f.repo, "");
    assert!(me.wrote(&author) && !me.known);

    f.repo
        .config()
        .unwrap()
        .set_str("user.name", "Ada")
        .unwrap();
    write(&f.work, "pages/a.md", "- a changed again\n");
    commit_now(&f.work, MESSAGE).unwrap();
    let author = head(&f.repo).author().to_owned();
    assert_eq!(
        (author.name().ok(), author.email().ok()),
        (Some("Ada"), Some(super::super::git_run::USER_FALLBACK.1))
    );
    assert!(super::super::git_run::user_identity(&f.repo, "").wrote(&author));
}
