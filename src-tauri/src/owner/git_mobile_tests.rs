//! Git on the phone (ADR 0078 slice 3), on every target: libgit2's writes on
//! the graph's own volume, a commit or push the OS ends midway, and the
//! worker's background and foreground edges.
//!
//! `TINE_HOST_FS_ROOT` (an existing directory) puts the graph on the volume
//! under test; app data, the remote and the other device stay in the private
//! temporary folder, as on a phone. CI runs this module on an Android
//! emulator with the graph on shared storage (MediaProvider's FUSE daemon,
//! GH #538-b) and `TINE_EXPECT_FUSE=1`, which fails unless the graph volume
//! is FUSE (`.github/workflows/android-git-storage.yml`).

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

    /// Another device, on case-sensitive private storage: clone, write
    /// (`Some`) or delete (`None`) each path, commit, push.
    fn peer_pushes(&self, changes: &[(&str, Option<&str>)]) -> Oid {
        let dir = self
            .private
            .join(format!("peer-{}", uuid::Uuid::new_v4().simple()));
        let peer = Repository::clone(&self.url, &dir).unwrap();
        configure(&peer);
        for (path, text) in changes {
            match text {
                Some(text) => fs::write(dir.join(path), text).unwrap(),
                None => fs::remove_file(dir.join(path)).unwrap(),
            }
        }
        let oid = commit_all(&peer, "from the laptop");
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
    fs::remove_dir_all(&probe).unwrap();
    if std::env::var("TINE_EXPECT_FUSE").as_deref() == Ok("1") {
        assert!(fuse, "the graph volume {} is not FUSE", dir.display());
    }
    format!(
        "volume={} fuse={fuse} noreplace={noreplace} folds_case={folds}",
        dir.display()
    )
}

/// Every object the repository holds reads back with its hash checked, every
/// ref names a commit that is there, and no lock file or index copy is left.
fn assert_whole(repo: &Repository, what: &str) -> Vec<String> {
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
                leftovers.push(name);
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

    // The laptop edits, creates, deletes and renames by case only; the
    // pull lands it all through the page host (GH #538-b: no-replace
    // publication on a volume that refuses the flag).
    let theirs = phone.peer_pushes(&[
        ("pages/a.md", Some("- one\n- two\n- from the laptop\n")),
        ("pages/new page.md", Some("- created on the laptop\n")),
        ("pages/gone.md", None),
        ("pages/c.md", None),
        ("pages/C.md", Some("- lower case\n")),
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
        ["C.md", "a.md", "b.md", "new page.md"],
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
    phone.edit("pages/C.md", "- lower case\n- edited after the pull\n");
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
    let mut stale_locks = 0;
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
        assert!(assert_whole(&phone.bare, "the remote").is_empty());
        if leftovers.iter().any(|l| l == "index.lock") {
            // ADR 0078 §1: Tine never removes it; the next commit is busy
            // until the person does (Settings names the file).
            stale_locks += 1;
            assert_eq!(commit_now(&phone.root, MESSAGE), Err(Unusable::Busy));
            fs::remove_file(phone.repo.path().join("index.lock")).unwrap();
        }
        // The next holder of the lock removes a killed index copy.
        assert!(matches!(
            commit_now(&phone.root, MESSAGE),
            Ok(Outcome::Committed(_) | Outcome::Clean)
        ));
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
        "GIT-KILL result=PASS kills={} stale_index_lock={stale_locks} {volume}",
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
    backgrounded();
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
    foregrounded();
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

#[test]
fn the_background_never_pulls_and_commit_only_never_pushes() {
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
    let theirs = phone.peer_pushes(&[("pages/a.md", Some("- from the laptop\n"))]);
    phone.edit("pages/b.md", "- typed before switching apps\n");
    backgrounded();
    wait_for("the background commit", || {
        last_attempt(&phone.root).commit.is_some()
    });
    let mine = head(&phone.repo);
    assert_ne!(mine, remote);
    assert_eq!(phone.remote_tip(), theirs, "commit only: nothing pushed");
    assert_eq!(phone.disk("pages/a.md"), "- one\n- two\n", "nothing pulled");
    foregrounded();
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
