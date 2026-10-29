//! Git on the phone (ADR 0078 slice 3), on every target: libgit2's writes on
//! the graph's own volume, a commit or push the OS ends midway, and the
//! worker's background and foreground edges.
//!
//! `TINE_HOST_FS_ROOT` (an existing directory) puts the graph on the volume
//! under test; app data, the remote and the other device stay in the private
//! temporary folder, as on a phone. CI runs this module on an Android
//! emulator with the graph on shared storage (MediaProvider's FUSE daemon,
//! GH #538-b) and `TINE_EXPECT_FUSE=1`, which fails unless the graph volume
//! is FUSE (`android-git-storage` in `ci.yml`, `.github/scripts/android-git-storage.sh`).

use super::*;
use crate::owner::git_sync::{NoCredentials, Operation, Outcome as Net, TransportOptions};
use crate::owner::git_sync_cycle::cycle;
use crate::state::{GraphSlot, PageHostSlot};
use git2::{Oid, Repository, Signature};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use tine_store::{Depth, PageHost, Store};

fn me() -> Signature<'static> {
    Signature::now("Martin", "martin@example.org").unwrap()
}

fn operation() -> Operation {
    Operation::new(
        Arc::new(NoCredentials),
        Arc::new(AtomicBool::new(false)),
        &TransportOptions::default(),
    )
    .unwrap()
}

fn credentials() -> Arc<dyn Credentials> {
    Arc::new(NoCredentials)
}

fn configure(repo: &Repository) {
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Martin").unwrap();
    config.set_str("user.email", "martin@example.org").unwrap();
    config.set_bool("core.autocrlf", false).unwrap();
    config.set_bool("core.precomposeunicode", true).unwrap();
}

/// Stage every change, deletions included, and commit it on `HEAD`.
fn commit_all(repo: &Repository, message: &str) -> Oid {
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.update_all(["*"], None).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let parent = repo.head().ok().map(|h| h.peel_to_commit().unwrap());
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(Some("HEAD"), &me(), &me(), message, &tree, &parents)
        .unwrap()
}

fn head(repo: &Repository) -> Oid {
    repo.head().unwrap().peel_to_commit().unwrap().id()
}

/// The graph on the volume under test, a clone-like repository on `main`
/// tracking a bare remote over `file://`, and the store and page host a
/// bound graph has.
struct Phone {
    root: PathBuf,
    app_data: PathBuf,
    private: PathBuf,
    store: Arc<Store>,
    repo: Repository,
    bare: Repository,
    url: String,
    _volume: tempfile::TempDir,
    _private: tempfile::TempDir,
}

impl Phone {
    fn new() -> Self {
        // As the app's openers do: on Android, shared storage reports files
        // as another uid's (CI run 40750744019 refused every repository).
        crate::owner::git_run::configure_libgit2();
        let private = tempfile::tempdir().unwrap();
        let volume = match std::env::var_os("TINE_HOST_FS_ROOT") {
            Some(base) => tempfile::Builder::new()
                .prefix("tine-git-")
                .tempdir_in(base)
                .unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let graph = volume.path().join("graph");
        fs::create_dir_all(graph.join("pages")).unwrap();
        fs::create_dir_all(graph.join("logseq")).unwrap();
        fs::write(
            graph.join("logseq/config.edn"),
            "{:file/name-format :triple-lowbar}\n",
        )
        .unwrap();
        fs::write(graph.join("pages/a.md"), "- one\n- two\n").unwrap();
        fs::write(graph.join("pages/b.md"), "- see [[a]]\n").unwrap();
        fs::write(graph.join("pages/c.md"), "- lower case\n").unwrap();
        fs::write(graph.join("pages/gone.md"), "- deleted elsewhere\n").unwrap();
        let app_data = private.path().join("app");
        fs::create_dir_all(&app_data).unwrap();
        let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        let root = Store::canonical_root(&graph).unwrap();

        let mut options = git2::RepositoryInitOptions::new();
        options.initial_head("main");
        let repo = Repository::init_opts(&root, &options).unwrap();
        configure(&repo);
        commit_all(&repo, "initial");
        let bare_path = private.path().join("remote.git");
        let bare = Repository::init_bare(&bare_path).unwrap();
        bare.set_head("refs/heads/main").unwrap();
        let url = reqwest::Url::from_file_path(&bare_path)
            .unwrap()
            .to_string();
        repo.remote("origin", &url).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("branch.main.remote", "origin").unwrap();
        config
            .set_str("branch.main.merge", "refs/heads/main")
            .unwrap();
        drop(config);
        assert_eq!(operation().push_upstream(&root), Net::UpToDate);
        // The first fetch records the remote-tracking ref a push compares with.
        assert_eq!(operation().fetch(&root), Net::UpToDate);
        Self {
            root,
            app_data,
            private: private.path().to_path_buf(),
            store,
            repo,
            bare,
            url,
            _volume: volume,
            _private: private,
        }
    }

    fn slot(&self) -> GraphSlot {
        let mut slot = GraphSlot::new(self.store.clone(), self.root.clone());
        let host = PageHost::start_for_tests(&self.store, &self.app_data).unwrap();
        *slot.host.get_mut().unwrap() = PageHostSlot::Running(host);
        slot
    }

    /// Another device: clone, write (`Some`) or delete (`None`) each path,
    /// commit, push. The commit is built in the index, not the clone's
    /// folder, so a case-only rename stays one even where the private
    /// temporary folder folds case (hosted macOS and Windows, run 40757889922).
    fn peer_pushes(&self, changes: &[(&str, Option<&str>)]) -> Oid {
        let dir = self
            .private
            .join(format!("peer-{}", uuid::Uuid::new_v4().simple()));
        let peer = Repository::clone(&self.url, &dir).unwrap();
        configure(&peer);
        peer.config()
            .unwrap()
            .set_bool("core.ignorecase", false)
            .unwrap();
        // Reopened, so the index is loaded under the setting just written.
        let peer = Repository::open(&dir).unwrap();
        let mut index = peer.index().unwrap();
        for (path, text) in changes {
            match text {
                Some(text) => {
                    let mut entry =
                        index
                            .get_path(Path::new(path), 0)
                            .unwrap_or(git2::IndexEntry {
                                ctime: git2::IndexTime::new(0, 0),
                                mtime: git2::IndexTime::new(0, 0),
                                dev: 0,
                                ino: 0,
                                mode: 0o100644,
                                uid: 0,
                                gid: 0,
                                file_size: 0,
                                id: Oid::ZERO_SHA1,
                                flags: 0,
                                flags_extended: 0,
                                path: Vec::new(),
                            });
                    entry.path = path.as_bytes().to_vec();
                    index.add_frombuffer(&entry, text.as_bytes()).unwrap();
                }
                None => index.remove_path(Path::new(path)).unwrap(),
            }
        }
        let tree = peer.find_tree(index.write_tree().unwrap()).unwrap();
        for (path, text) in changes {
            assert_eq!(
                tree.get_path(Path::new(path)).is_ok(),
                text.is_some(),
                "the peer's commit has exactly the spelling {path}"
            );
        }
        let parent = peer.head().unwrap().peel_to_commit().unwrap();
        let oid = peer
            .commit(
                Some("HEAD"),
                &me(),
                &me(),
                "from the laptop",
                &tree,
                &[&parent],
            )
            .unwrap();
        assert_eq!(operation().push_upstream(&dir), Net::UpToDate);
        oid
    }

    fn remote_tip(&self) -> Oid {
        self.bare
            .find_reference("refs/heads/main")
            .unwrap()
            .target()
            .unwrap()
    }

    /// An edit the store sees, as a save or the watcher would publish it.
    fn edit(&self, path: &str, text: &str) {
        fs::write(self.root.join(path), text).unwrap();
        self.store.refresh(Depth::Bytes).unwrap();
    }

    fn disk(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).unwrap()
    }

    fn page_names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.root.join("pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

/// What the graph volume is, printed for the CI log and checked against
/// `TINE_EXPECT_FUSE`: FUSE or not, whether it answers the no-replace rename
/// flag (GH #538-b: MediaProvider before Android 15 refuses it), and whether
/// it folds case.
fn describe_volume(dir: &Path) -> String {
    let probe = dir.join("probe");
    fs::create_dir_all(&probe).unwrap();
    fs::write(probe.join("fold"), "x").unwrap();
    let folds = probe.join("FOLD").exists();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let (fuse, noreplace) = {
        use std::os::unix::ffi::OsStrExt;
        const FUSE_SUPER_MAGIC: i64 = 0x6573_5546;
        let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::statfs(c(&probe).as_ptr(), &mut stat) }, 0);
        fs::write(probe.join("from"), "x").unwrap();
        let renamed = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                c(&probe.join("from")).as_ptr(),
                libc::AT_FDCWD,
                c(&probe.join("to")).as_ptr(),
                libc::RENAME_NOREPLACE as libc::c_uint,
            )
        };
        let noreplace = match renamed {
            0 => "works".to_string(),
            _ => format!("refused ({})", std::io::Error::last_os_error()),
        };
        (stat.f_type as i64 == FUSE_SUPER_MAGIC, noreplace)
    };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let (fuse, noreplace) = (false, "not probed".to_string());
    // A case-only rename (a pull that respells a page): does a plain rename
    // land, and which spelling does the directory list afterwards?
    let respell = match fs::rename(probe.join("fold"), probe.join("Fold")) {
        Ok(()) => {
            let mut listed: Vec<String> = fs::read_dir(&probe)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            listed.sort();
            format!("renamed,listed:{}", listed.join("|"))
        }
        Err(e) => format!("refused ({e})"),
    };
    fs::remove_dir_all(&probe).unwrap();
    if std::env::var("TINE_EXPECT_FUSE").as_deref() == Ok("1") {
        assert!(fuse, "the graph volume {} is not FUSE", dir.display());
    }
    format!(
        "volume={} fuse={fuse} noreplace={noreplace} folds_case={folds} case_rename={respell}",
        dir.display()
    )
}

/// Every object the repository holds reads back with its hash checked, every
/// ref names a commit that is there, and no lock file or index copy is left.
fn assert_whole(repo: &Repository, what: &str) -> Vec<PathBuf> {
    let odb = repo.odb().unwrap();
    let mut oids = Vec::new();
    odb.foreach(|oid| {
        oids.push(*oid);
        true
    })
    .unwrap();
    for oid in oids {
        odb.read(oid)
            .unwrap_or_else(|e| panic!("{what}: object {oid} unreadable: {e}"));
    }
    for reference in repo.references().unwrap() {
        let reference = reference.unwrap();
        let Some(target) = reference.resolve().unwrap().target() else {
            continue;
        };
        let commit = repo
            .find_commit(target)
            .unwrap_or_else(|e| panic!("{what}: {:?} -> {target}: {e}", reference.name()));
        let mut walk = repo.revwalk().unwrap();
        walk.push(commit.id()).unwrap();
        for oid in walk {
            let tree = repo.find_commit(oid.unwrap()).unwrap().tree().unwrap();
            tree.walk(git2::TreeWalkMode::PreOrder, |_, entry| {
                assert!(
                    entry.to_object(repo).is_ok(),
                    "{what}: tree entry {:?} missing",
                    entry.name()
                );
                git2::TreeWalkResult::Ok
            })
            .unwrap();
        }
    }
    if !repo.is_bare() {
        repo.index().unwrap();
    }
    let mut leftovers = Vec::new();
    let mut dirs = vec![repo.path().to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().unwrap().is_dir() {
                dirs.push(entry.path());
            } else if name.ends_with(".lock") || name.starts_with("tine-index-") {
                leftovers.push(entry.path());
            }
        }
    }
    leftovers
}

#[test]
fn git_commits_pulls_pushes_and_repacks_on_the_graph_volume() {
    let phone = Phone::new();
    let volume = describe_volume(phone.root.parent().unwrap());
    let slot = phone.slot();

    // This device commits and pushes.
    phone.edit("pages/b.md", "- typed on the phone\n");
    assert!(matches!(
        commit_now(&phone.root, MESSAGE),
        Ok(Outcome::Committed(_))
    ));
    let pushed = cycle(&slot, &credentials());
    assert_eq!(pushed.code, "pushed", "{pushed:?}");
    assert_eq!(phone.remote_tip(), head(&phone.repo));

    // The laptop edits, creates and deletes; the pull lands it all through
    // the page host (GH #538-b: no-replace publication on a volume that
    // refuses the flag).
    let theirs = phone.peer_pushes(&[
        ("pages/a.md", Some("- one\n- two\n- from the laptop\n")),
        ("pages/new page.md", Some("- created on the laptop\n")),
        ("pages/gone.md", None),
    ]);
    let pulled = cycle(&slot, &credentials());
    assert_eq!(pulled.code, "pulled", "{pulled:?} on {volume}");
    assert_eq!(head(&phone.repo), theirs);
    assert_eq!(
        phone.disk("pages/a.md"),
        "- one\n- two\n- from the laptop\n"
    );
    assert_eq!(phone.disk("pages/new page.md"), "- created on the laptop\n");
    assert_eq!(
        phone.page_names(),
        ["a.md", "b.md", "c.md", "new page.md"],
        "{volume}"
    );
    let statuses = phone
        .repo
        .statuses(Some(git2::StatusOptions::new().include_untracked(true)))
        .unwrap();
    let dirty: Vec<_> = statuses
        .iter()
        .filter_map(|s| s.path().ok().map(str::to_owned))
        .filter(|p| p.starts_with("pages/"))
        .collect();
    assert!(dirty.is_empty(), "{dirty:?} on {volume}");

    // Another edit after the pull, then the repack writes its pack here.
    phone.edit("pages/c.md", "- lower case\n- edited after the pull\n");
    assert!(matches!(
        commit_now(&phone.root, MESSAGE),
        Ok(Outcome::Committed(_))
    ));
    assert!(repack(&phone.repo).unwrap().is_some_and(|n| n > 0));
    assert_eq!(cycle(&slot, &credentials()).code, "pushed");
    assert_eq!(cycle(&slot, &credentials()).code, "up-to-date");
    let leftovers = assert_whole(&phone.repo, "the phone's repository");
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(assert_whole(&phone.bare, "the remote").is_empty());
    eprintln!("GIT-STORAGE result=PASS {volume}");
}

/// GH #538-b and case folding: the laptop renames a page by case only.
/// Fails on Android shared storage (MediaProvider keeps the old spelling)
/// until GS-S1 is decided; the assertion stays, and the `android-git-storage`
/// CI job stays out of `scope=full` meanwhile.
#[test]
fn a_pulled_case_only_rename_lands_on_the_graph_volume() {
    let phone = Phone::new();
    let volume = describe_volume(phone.root.parent().unwrap());
    let slot = phone.slot();
    let theirs = phone.peer_pushes(&[("pages/c.md", None), ("pages/C.md", Some("- lower case\n"))]);
    let pulled = cycle(&slot, &credentials());
    if pulled.code != "pulled" {
        // What the stopped pull left, for the CI log, before the assert.
        let pages: Vec<_> = (phone.page_names().into_iter())
            .map(|n| {
                (
                    fs::read_to_string(phone.root.join("pages").join(&n)).ok(),
                    n,
                )
            })
            .collect();
        let at_theirs = head(&phone.repo) == theirs;
        let again = cycle(&slot, &credentials());
        let leftovers = assert_whole(&phone.repo, "after the stopped pull");
        eprintln!(
            "GIT-RESPELL stopped pull: pages={pages:?} head_at_theirs={at_theirs} \
             retry={again:?} leftovers={leftovers:?}"
        );
    }
    assert_eq!(pulled.code, "pulled", "{pulled:?} on {volume}");
    assert_eq!(head(&phone.repo), theirs);
    assert_eq!(
        phone.page_names(),
        ["C.md", "a.md", "b.md", "gone.md"],
        "{volume}"
    );
    assert_eq!(phone.disk("pages/C.md"), "- lower case\n");
    assert!(assert_whole(&phone.repo, "the phone's repository").is_empty());
    eprintln!("GIT-RESPELL result=PASS {volume}");
}

/// A commit the OS ended can leave the branch's lock as well as the index's:
/// Settings names the one actually held, not always `index.lock`.
#[test]
fn settings_names_the_lock_a_killed_commit_left() {
    let phone = Phone::new();
    assert_eq!(held_lock(&phone.root), None);
    let branch = phone.repo.path().join("refs/heads/main.lock");
    fs::write(&branch, "").unwrap();
    phone.edit("pages/b.md", "- typed before the kill\n");
    assert_eq!(commit_now(&phone.root, MESSAGE), Err(Unusable::Busy));
    let named = held_lock(&phone.root).unwrap();
    assert!(named.ends_with("refs/heads/main.lock"), "{named}");
    fs::write(phone.repo.path().join("index.lock"), "").unwrap();
    assert!(held_lock(&phone.root).unwrap().ends_with("index.lock"));
    fs::remove_file(phone.repo.path().join("index.lock")).unwrap();
    fs::remove_file(branch).unwrap();
    assert!(matches!(
        commit_now(&phone.root, MESSAGE),
        Ok(Outcome::Committed(_))
    ));
}

/// The child half of the kill test: commit and push until the parent ends
/// this process.
fn commit_and_push_until_killed(root: &Path) {
    fs::write(root.join("ready"), "").unwrap();
    for i in 0.. {
        let text: String = (0..200).map(|n| format!("- edit {i} line {n}\n")).collect();
        fs::write(root.join("pages/b.md"), text).unwrap();
        commit_now(root, MESSAGE).unwrap();
        if i % 2 == 0 {
            let _ = operation().push_upstream(root);
        }
    }
}

#[test]
fn a_commit_or_push_the_os_ends_leaves_the_repository_whole() {
    if let Some(root) = std::env::var_os("TINE_GIT_KILL_CHILD") {
        commit_and_push_until_killed(Path::new(&root));
        return;
    }
    let phone = Phone::new();
    let volume = describe_volume(phone.root.parent().unwrap());
    fs::write(phone.root.join(".gitignore"), "/ready\n").unwrap();
    commit_all(&phone.repo, "ignore the child's marker");
    let name = format!(
        "{}::a_commit_or_push_the_os_ends_leaves_the_repository_whole",
        module_path!().split_once("::").unwrap().1
    );
    let mut stale_locks = std::collections::BTreeMap::<String, u32>::new();
    let mut remote_locks = 0;
    let delays = [0u64, 2, 5, 9, 14, 20, 30, 45, 70, 110, 170, 260, 400];
    for delay in delays {
        let _ = fs::remove_file(phone.root.join("ready"));
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture", "--test-threads=1"])
            .env("TINE_GIT_KILL_CHILD", &phone.root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while !phone.root.join("ready").exists() {
            assert!(Instant::now() < deadline, "the child never started");
            assert!(child.try_wait().unwrap().is_none(), "the child exited");
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(delay));
        // SIGKILL on Unix, TerminateProcess on Windows: as the OS ends a
        // backgrounded app, with no unwinding and no destructors.
        child.kill().unwrap();
        child.wait().unwrap();

        let leftovers = assert_whole(&phone.repo, &format!("killed after {delay} ms"));
        // The remote is a file:// repository this process wrote as a server
        // would; a server recovers its own locks. Its objects and refs must
        // be whole.
        for lock in assert_whole(&phone.bare, "the remote") {
            remote_locks += 1;
            fs::remove_file(lock).unwrap();
        }
        // ADR 0078 §1: Tine never removes a lock unasked; the next commit is
        // busy and Settings names the lock until the person removes it. An
        // `index.lock` a kill left is Tine's own, empty one (GS-LOCK's second
        // condition), and once it is old, the click removes it.
        for _ in 0..4 {
            let next = commit_now(&phone.root, MESSAGE);
            if next != Err(Unusable::Busy) {
                assert!(
                    matches!(next, Ok(Outcome::Committed(_) | Outcome::Clean)),
                    "killed after {delay} ms, left {leftovers:?}: the next commit {next:?} on {volume}"
                );
                break;
            }
            let named = held_lock(&phone.root).map(PathBuf::from);
            let lock = named.unwrap_or_else(|| {
                panic!("killed after {delay} ms, left {leftovers:?}: busy, no lock named")
            });
            assert!(leftovers.contains(&lock), "{lock:?} not in {leftovers:?}");
            *stale_locks
                .entry(lock.file_name().unwrap().to_string_lossy().into_owned())
                .or_insert(0) += 1;
            if lock.file_name() == Some(std::ffi::OsStr::new("index.lock")) {
                assert_eq!(
                    fs::metadata(&lock).unwrap().len(),
                    0,
                    "killed after {delay} ms: a non-empty index.lock on {volume}"
                );
                let later = SystemTime::now() + STALE_AFTER;
                let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
                let offer = stale_lock::assess(&phone.root, later)
                    .unwrap_or_else(|| panic!("killed after {delay} ms: not offered"));
                drop(_run);
                assert_eq!(
                    stale_lock::remove_within(
                        &phone.root,
                        &offer.token,
                        Duration::from_secs(60),
                        later
                    ),
                    Ok(Removed::Removed),
                    "killed after {delay} ms on {volume}"
                );
            } else {
                fs::remove_file(lock).unwrap();
            }
        }
        // The next holder of the index lock removes a killed index copy.
        let leftovers = assert_whole(&phone.repo, "after the next commit");
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
    let slot = phone.slot();
    let last = cycle(&slot, &credentials());
    assert!(
        matches!(last.code, "pushed" | "up-to-date"),
        "{last:?} on {volume}"
    );
    assert_eq!(phone.remote_tip(), head(&phone.repo));
    eprintln!(
        "GIT-KILL result=PASS kills={} stale_locks={stale_locks:?} remote_locks={remote_locks} {volume}",
        delays.len()
    );
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_background_commits_and_pushes_and_the_foreground_syncs() {
    let phone = Phone::new();
    let slot = Arc::new(phone.slot());
    start_worker(
        &slot,
        GitSettings {
            policy: Policy::SyncAutomatically,
            interval: Duration::from_secs(10 * 60),
        },
        credentials(),
    );
    // The worker's opening cycle finds nothing to do.
    wait_for("the opening cycle", || {
        let sync = last_attempt(&phone.root).sync;
        !sync.running && sync.at.is_some()
    });

    // Typed, saved, then the app goes to the background: one commit and its
    // push, long before the ten-minute cadence.
    let before = head(&phone.repo);
    phone.edit("pages/b.md", "- typed before switching apps\n");
    backgrounded_at(&phone.root);
    wait_for("the background push", || {
        head(&phone.repo) != before && phone.remote_tip() == head(&phone.repo)
    });
    let tip = phone.bare.find_commit(phone.remote_tip()).unwrap();
    assert_eq!(tip.message().ok(), Some(MESSAGE));
    let blob = tip
        .tree()
        .unwrap()
        .get_path(Path::new("pages/b.md"))
        .unwrap();
    assert_eq!(
        phone.bare.find_blob(blob.id()).unwrap().content(),
        b"- typed before switching apps\n"
    );

    // The laptop pushes while the phone sleeps; back in the foreground the
    // phone pulls at once.
    let theirs = phone.peer_pushes(&[("pages/a.md", Some("- from the laptop\n"))]);
    foregrounded_at(&phone.root);
    wait_for("the foreground pull", || head(&phone.repo) == theirs);
    assert_eq!(phone.disk("pages/a.md"), "- from the laptop\n");
    wait_for("the cycle's end", || {
        !last_attempt(&phone.root).sync.running
    });
    assert_eq!(
        last_attempt(&phone.root).sync.code.as_deref(),
        Some("pulled")
    );
    slot.background_cancelled.store(true, Ordering::Release);
}

/// Autocommits on this branch since `from` (exclusive).
fn autocommits_since(repo: &Repository, from: Oid) -> usize {
    let mut walk = repo.revwalk().unwrap();
    walk.push(head(repo)).unwrap();
    walk.hide(from).unwrap();
    walk.map(|oid| repo.find_commit(oid.unwrap()).unwrap())
        .filter(|c| c.message().ok() == Some(MESSAGE))
        .count()
}

/// D-20261026-GC caps autocommits at one per interval (≤ 6 an hour at the
/// default ten minutes); switching apps is no exemption, and neither is
/// coming back. Edits the cap holds back commit once the interval allows.
#[test]
fn switching_apps_commits_at_most_once_per_interval() {
    let phone = Phone::new();
    let slot = Arc::new(phone.slot());
    let interval = Duration::from_secs(20);
    start_worker(
        &slot,
        GitSettings {
            policy: Policy::SyncAutomatically,
            interval,
        },
        credentials(),
    );
    wait_for("the opening cycle", || {
        let sync = last_attempt(&phone.root).sync;
        !sync.running && sync.at.is_some()
    });
    let start = head(&phone.repo);
    phone.edit("pages/b.md", "- switch 0\n");
    backgrounded_at(&phone.root);
    wait_for("the first background commit", || head(&phone.repo) != start);
    let first = Instant::now();
    // Seven more app switches with an edit each, all inside one interval.
    for n in 1..=7 {
        foregrounded_at(&phone.root);
        std::thread::sleep(Duration::from_millis(300));
        phone.edit("pages/b.md", &format!("- switch {n}\n"));
        backgrounded_at(&phone.root);
        std::thread::sleep(Duration::from_millis(300));
    }
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        first.elapsed() < interval,
        "the switches outlasted the interval"
    );
    assert_eq!(autocommits_since(&phone.repo, start), 1, "one per interval");
    assert_eq!(
        phone.disk("pages/b.md"),
        "- switch 7\n",
        "the edit is saved"
    );

    // Once the interval has passed, the next switch commits what waited.
    std::thread::sleep(interval.saturating_sub(first.elapsed()) + Duration::from_secs(1));
    backgrounded_at(&phone.root);
    wait_for("the held-back edits' commit", || {
        autocommits_since(&phone.repo, start) == 2
    });
    let tip = phone.repo.find_commit(head(&phone.repo)).unwrap();
    let blob = tip
        .tree()
        .unwrap()
        .get_path(Path::new("pages/b.md"))
        .unwrap();
    assert_eq!(
        phone.repo.find_blob(blob.id()).unwrap().content(),
        b"- switch 7\n"
    );
    slot.background_cancelled.store(true, Ordering::Release);
}

/// Backgrounded stays backgrounded until the foreground edge (review F1):
/// the automatic cycle, with its fetch and pull, waits however long the
/// app is away, and runs as soon as it comes back.
#[test]
fn the_background_never_fetches_or_pulls() {
    let phone = Phone::new();
    let slot = Arc::new(phone.slot());
    let interval = Duration::from_secs(3);
    start_worker(
        &slot,
        GitSettings {
            policy: Policy::SyncAutomatically,
            interval,
        },
        credentials(),
    );
    wait_for("the opening cycle", || {
        let sync = last_attempt(&phone.root).sync;
        !sync.running && sync.at.is_some()
    });
    let tracking = || {
        phone
            .repo
            .refname_to_id("refs/remotes/origin/main")
            .unwrap()
    };
    let (mine, seen) = (head(&phone.repo), tracking());
    let theirs = phone.peer_pushes(&[("pages/a.md", Some("- from the laptop\n"))]);
    backgrounded_at(&phone.root);
    // Past the interval and a worker tick: an automatic cycle would be due.
    std::thread::sleep(interval + TICK + Duration::from_secs(3));
    assert_eq!(head(&phone.repo), mine, "nothing pulled in the background");
    assert_eq!(tracking(), seen, "nothing fetched in the background");
    assert_eq!(phone.disk("pages/a.md"), "- one\n- two\n");

    foregrounded_at(&phone.root);
    wait_for("the foreground pull", || head(&phone.repo) == theirs);
    assert_eq!(phone.disk("pages/a.md"), "- from the laptop\n");
    slot.background_cancelled.store(true, Ordering::Release);
}

/// A worker that starts while the app is away starts away (review r2 B1):
/// a bind that ends after the person switched apps pulls nothing until the
/// app comes back.
#[test]
fn a_worker_started_while_away_waits_for_the_foreground() {
    let phone = Phone::new();
    let slot = Arc::new(phone.slot());
    let tracking = || {
        phone
            .repo
            .refname_to_id("refs/remotes/origin/main")
            .unwrap()
    };
    let (mine, seen) = (head(&phone.repo), tracking());
    let theirs = phone.peer_pushes(&[("pages/a.md", Some("- from the laptop\n"))]);
    backgrounded_at(&phone.root);
    start_worker(
        &slot,
        GitSettings {
            policy: Policy::SyncAutomatically,
            interval: Duration::from_secs(3),
        },
        credentials(),
    );
    // The opening cycle and a tick past the interval: both would pull.
    std::thread::sleep(TICK + Duration::from_secs(5));
    assert_eq!(head(&phone.repo), mine, "nothing pulled in the background");
    assert_eq!(tracking(), seen, "nothing fetched in the background");
    assert_eq!(phone.disk("pages/a.md"), "- one\n- two\n");

    foregrounded_at(&phone.root);
    wait_for("the foreground pull", || head(&phone.repo) == theirs);
    slot.background_cancelled.store(true, Ordering::Release);
}

/// "Sync now" pressed as the app leaves (review r2 FU1): the request waits
/// for the foreground, neither syncing in the background nor getting lost.
#[test]
fn a_sync_asked_for_while_away_runs_in_the_foreground() {
    let phone = Phone::new();
    let slot = Arc::new(phone.slot());
    start_worker(
        &slot,
        GitSettings {
            policy: Policy::SyncOnDemand,
            interval: Duration::from_secs(10 * 60),
        },
        credentials(),
    );
    let mine = head(&phone.repo);
    let theirs = phone.peer_pushes(&[("pages/a.md", Some("- from the laptop\n"))]);
    backgrounded_at(&phone.root);
    assert!(request_sync(&phone.root));
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(head(&phone.repo), mine, "nothing pulled in the background");
    assert_eq!(phone.disk("pages/a.md"), "- one\n- two\n");

    // On demand, coming back syncs only because the request waited.
    foregrounded_at(&phone.root);
    wait_for("the requested pull", || head(&phone.repo) == theirs);
    assert_eq!(phone.disk("pages/a.md"), "- from the laptop\n");
    slot.background_cancelled.store(true, Ordering::Release);
}

#[test]
fn commit_only_never_pushes_from_the_background() {
    let phone = Phone::new();
    let slot = Arc::new(phone.slot());
    start_worker(
        &slot,
        GitSettings {
            policy: Policy::Commit,
            interval: Duration::from_secs(10 * 60),
        },
        credentials(),
    );
    let remote = phone.remote_tip();
    phone.edit("pages/b.md", "- typed before switching apps\n");
    backgrounded_at(&phone.root);
    wait_for("the background commit", || {
        last_attempt(&phone.root).commit.is_some()
    });
    // A push would follow the commit at once.
    std::thread::sleep(Duration::from_secs(2));
    let mine = head(&phone.repo);
    assert_ne!(mine, remote);
    assert_eq!(phone.remote_tip(), remote, "commit only: nothing pushed");
    foregrounded_at(&phone.root);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        head(&phone.repo),
        mine,
        "the foreground syncs only automatic sync"
    );
    assert!(
        crate::owner::git_sync::push_pending(&phone.root),
        "the commit waits for a sync the person starts"
    );
    slot.background_cancelled.store(true, Ordering::Release);
}

// ---- a stale index.lock (GS-LOCK) -------------------------------------------------

use crate::owner::git_stale_lock::{self as stale_lock, Refusal, Removed, StaleLock, STALE_AFTER};
use std::time::SystemTime;

/// An empty `index.lock`, as a commit the OS ended leaves it, last changed
/// `age` ago.
fn left_lock(phone: &Phone, age: Duration) -> PathBuf {
    let lock = phone.repo.path().join("index.lock");
    fs::write(&lock, "").unwrap();
    set_age(&lock, age);
    lock
}

fn set_age(path: &Path, age: Duration) {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - age)
        .unwrap();
}

/// The offer as the worker computes it, holding RUN_LOCK for as long as
/// another test's commit takes (the worker's own check gives up after
/// 250 ms instead).
fn offered(root: &Path) -> Option<StaleLock> {
    let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    stale_lock::assess(root, SystemTime::now())
}

fn click(root: &Path, token: &str) -> Result<Removed, Refusal> {
    stale_lock::remove_within(root, token, Duration::from_secs(60), SystemTime::now())
}

/// Before GS-LOCK a lock a killed commit left kept every commit busy until
/// someone removed it by hand. Now Tine offers it, and the click recovers the
/// commit.
#[test]
fn a_stale_index_lock_is_offered_and_removing_it_recovers_the_commit() {
    let phone = Phone::new();
    let lock = left_lock(&phone, STALE_AFTER + Duration::from_secs(60));
    phone.edit("pages/b.md", "- typed before the kill\n");
    assert_eq!(commit_now(&phone.root, MESSAGE), Err(Unusable::Busy));
    let offer = offered(&phone.root).expect("a stale empty lock is offered");
    assert_eq!(PathBuf::from(&offer.path), lock);
    assert_eq!(click(&phone.root, &offer.token), Ok(Removed::Removed));
    assert!(!lock.exists());
    let leftovers: Vec<_> = fs::read_dir(phone.repo.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("tine-stale-index-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(matches!(
        commit_now(&phone.root, MESSAGE),
        Ok(Outcome::Committed(_))
    ));
    assert_eq!(offered(&phone.root), None);
}

#[test]
fn a_fresh_or_future_index_lock_is_not_offered() {
    let phone = Phone::new();
    let lock = left_lock(&phone, Duration::ZERO);
    assert_eq!(offered(&phone.root), None, "just created");
    set_age(&lock, STALE_AFTER - Duration::from_secs(60));
    assert_eq!(offered(&phone.root), None, "nine minutes old");
    fs::OpenOptions::new()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
    assert_eq!(offered(&phone.root), None, "a time in the future");
    assert_eq!(commit_now(&phone.root, MESSAGE), Err(Unusable::Busy));
    assert!(lock.exists());
}

/// A git program writes the new index into its lock; `git commit -a` holds
/// that lock for as long as its editor is open. Old or not, it is not offered.
#[test]
fn an_index_lock_with_content_is_not_offered() {
    let phone = Phone::new();
    let lock = phone.repo.path().join("index.lock");
    fs::copy(phone.repo.path().join("index"), &lock).unwrap();
    set_age(&lock, STALE_AFTER * 6);
    assert_eq!(offered(&phone.root), None);
    assert!(lock.exists());
}

#[test]
fn no_offer_and_no_removal_while_tine_runs_a_git_operation() {
    let phone = Phone::new();
    let lock = left_lock(&phone, STALE_AFTER * 2);
    let token = offered(&phone.root).unwrap().token;
    let (held, release) = (
        std::sync::mpsc::channel::<()>(),
        std::sync::mpsc::channel::<()>(),
    );
    let (held_tx, held_rx) = held;
    let (release_tx, release_rx) = release;
    let operation = std::thread::spawn(move || {
        let _run = RUN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        held_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    held_rx.recv().unwrap();
    assert_eq!(stale_lock::offer(&phone.root), None);
    assert_eq!(
        stale_lock::remove_within(
            &phone.root,
            &token,
            Duration::from_millis(100),
            SystemTime::now()
        ),
        Err(Refusal::TineBusy)
    );
    assert!(lock.exists());
    release_tx.send(()).unwrap();
    operation.join().unwrap();
    assert_eq!(click(&phone.root, &token), Ok(Removed::Removed));
}

/// The click rechecks the lock it was offered: one changed since (a git
/// program took it up again), even one still old, stays.
#[test]
fn the_click_refuses_a_lock_that_changed_since_the_offer() {
    let phone = Phone::new();
    let lock = left_lock(&phone, STALE_AFTER * 2);
    let token = offered(&phone.root).unwrap().token;
    set_age(&lock, STALE_AFTER + Duration::from_secs(30));
    assert_eq!(click(&phone.root, &token), Err(Refusal::Changed));
    assert!(lock.exists());
    set_age(&lock, Duration::ZERO);
    assert_eq!(click(&phone.root, &token), Err(Refusal::Changed));
    assert!(lock.exists());
    fs::write(&lock, "a git program's new index").unwrap();
    set_age(&lock, STALE_AFTER * 2);
    let now = offered(&phone.root);
    assert_eq!(now, None, "content: not offered any more");
    // A lock someone else removed meanwhile is simply gone.
    fs::remove_file(&lock).unwrap();
    assert_eq!(click(&phone.root, &token), Ok(Removed::Gone));
}

/// Only `index.lock` is ever offered or removed: a branch lock a killed
/// commit left stays for the person, and Settings goes on naming it.
#[test]
fn removing_a_stale_index_lock_leaves_ref_locks_alone() {
    let phone = Phone::new();
    let branch = phone.repo.path().join("refs/heads/main.lock");
    fs::write(&branch, "").unwrap();
    set_age(&branch, STALE_AFTER * 2);
    assert_eq!(offered(&phone.root), None, "a branch lock alone");
    left_lock(&phone, STALE_AFTER * 2);
    let token = offered(&phone.root).unwrap().token;
    assert_eq!(click(&phone.root, &token), Ok(Removed::Removed));
    assert!(branch.exists());
    phone.edit("pages/b.md", "- typed before the kill\n");
    assert_eq!(commit_now(&phone.root, MESSAGE), Err(Unusable::Busy));
    assert!(held_lock(&phone.root)
        .unwrap()
        .ends_with("refs/heads/main.lock"));
    fs::remove_file(branch).unwrap();
}

/// End to end on the worker: the phone goes to the background, the commit
/// finds the lock a killed commit left, the notice offers it under the
/// commit-only policy too, and after the click the worker commits at once.
#[test]
fn the_worker_offers_a_stale_lock_and_commits_after_the_click() {
    let phone = Phone::new();
    let lock = left_lock(&phone, STALE_AFTER * 2);
    let slot = Arc::new(phone.slot());
    start_worker(
        &slot,
        GitSettings {
            policy: Policy::Commit,
            interval: Duration::from_secs(10 * 60),
        },
        credentials(),
    );
    let before = head(&phone.repo);
    phone.edit("pages/b.md", "- typed before switching apps\n");
    backgrounded_at(&phone.root);
    wait_for("the offer", || {
        last_attempt(&phone.root).stale_lock_offer.is_some()
    });
    foregrounded_at(&phone.root);
    let notice = crate::git_settings::notice_for(Policy::Commit, &phone.root).unwrap();
    assert_eq!(notice.code.as_deref(), Some("stale-lock"));
    assert!(notice.persistent);
    let offer = notice.stale_lock.unwrap();
    assert_eq!(PathBuf::from(&offer.path), lock);
    assert_eq!(
        crate::git_settings::notice_for(Policy::Off, &phone.root),
        None
    );
    assert_eq!(
        stale_lock::remove(&phone.root, &offer.token),
        Ok(Removed::Removed)
    );
    assert!(lock_removed(&phone.root, false));
    wait_for("the commit after the click", || head(&phone.repo) != before);
    wait_for("the offer to clear", || {
        last_attempt(&phone.root).stale_lock_offer.is_none()
    });
    assert_eq!(
        crate::git_settings::notice_for(Policy::Commit, &phone.root),
        None
    );
    slot.background_cancelled.store(true, Ordering::Release);
}
