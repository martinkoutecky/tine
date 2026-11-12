//! GH #872: a graph whose `.git` is a file naming a separate git directory.
//! Logseq desktop's version control makes this layout itself when a graph has
//! no `.git` folder (`git init --separate-git-dir` into
//! `~/.logseq/git/<graph>/.git`, `electron/git.cljs` `init!`), so it is the
//! normal state of a graph that had Logseq's version control on. git reports
//! the folder holding the `.git` file as the work tree; Tine must too, commit
//! and sync it like any other, write nothing but git's own metadata into the
//! separate directory, and never convert the layout.

use super::*;
use crate::owner::git_sync::{NoCredentials, Operation, Outcome as Net, TransportOptions};
use crate::state::{GraphSlot, PageHostSlot};
use git2::{Repository, Signature};
use std::fs;
use tine_store::{Depth, PageHost, Store};

#[derive(Clone, Copy, Debug)]
enum Layout {
    /// `git init --separate-git-dir`, as Logseq runs it.
    GitCli,
    /// A `.git` file with an absolute `gitdir:` (the reporter's file).
    Absolute,
    /// A `.git` file with a relative `gitdir:`, resolved from the graph.
    Relative,
}

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

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Run the git command line hermetically (no user or system config).
fn git(args: &[&std::ffi::OsStr]) {
    let output = std::process::Command::new("git")
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .expect("the git command line runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A home folder holding the graph `logseq/` with its git directory at
/// `.logseq/git/_home_logseq/.git`, as Logseq lays it out, a bare remote it
/// tracks over `file://`, and the store and page host a bound graph has.
struct Graph {
    root: PathBuf,
    gitdir: PathBuf,
    gitfile: Vec<u8>,
    app_data: PathBuf,
    store: Arc<Store>,
    repo: Repository,
    bare: Repository,
    url: String,
    home: tempfile::TempDir,
    _private: tempfile::TempDir,
}

impl Graph {
    fn new(layout: Layout) -> Self {
        configure_libgit2_for_tests();
        let home = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let graph = home.path().join("logseq");
        fs::create_dir_all(graph.join("pages")).unwrap();
        fs::create_dir_all(graph.join("logseq")).unwrap();
        fs::write(
            graph.join("logseq/config.edn"),
            "{:file/name-format :triple-lowbar}\n",
        )
        .unwrap();
        fs::write(graph.join("pages/a.md"), "- one\n").unwrap();
        fs::write(graph.join("pages/b.md"), "- see [[a]]\n").unwrap();
        let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        let root = Store::canonical_root(&graph).unwrap();
        // Paths as the user types them (on Windows the canonical root is a
        // verbatim `\\?\` path, which neither git nor a gitfile uses).
        let container = home.path().join(".logseq/git/_home_logseq");
        fs::create_dir_all(&container).unwrap();
        let gitdir = container.join(".git");
        match layout {
            Layout::GitCli => {
                let mut separate = std::ffi::OsString::from("--separate-git-dir=");
                separate.push(&gitdir);
                git(&[
                    "init".as_ref(),
                    "-q".as_ref(),
                    "--initial-branch=main".as_ref(),
                    &separate,
                    graph.as_os_str(),
                ]);
            }
            Layout::Absolute | Layout::Relative => {
                let mut options = git2::RepositoryInitOptions::new();
                options.initial_head("main");
                drop(Repository::init_opts(&root, &options).unwrap());
                fs::rename(root.join(".git"), &gitdir).unwrap();
                let target = match layout {
                    Layout::Absolute => gitdir.to_string_lossy().replace('\\', "/"),
                    _ => "../.logseq/git/_home_logseq/.git".to_owned(),
                };
                fs::write(root.join(".git"), format!("gitdir: {target}\n")).unwrap();
            }
        }
        assert!(root.join(".git").is_file(), "{layout:?}: a .git file");
        let gitfile = fs::read(root.join(".git")).unwrap();
        let repo = Repository::open(&root).unwrap();
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Martin").unwrap();
            config.set_str("user.email", "martin@example.org").unwrap();
            config.set_bool("core.autocrlf", false).unwrap();
            config.set_bool("core.precomposeunicode", true).unwrap();
        }
        assert!(
            repo.config().unwrap().get_path("core.worktree").is_err(),
            "{layout:?}: git's layout has no core.worktree"
        );
        commit_all(&repo, "initial");
        let bare_path = private.path().join("remote.git");
        let bare = Repository::init_bare(&bare_path).unwrap();
        bare.set_head("refs/heads/main").unwrap();
        let url = reqwest::Url::from_file_path(&bare_path)
            .unwrap()
            .to_string();
        repo.remote("origin", &url).unwrap();
        {
            let mut config = repo.config().unwrap();
            config.set_str("branch.main.remote", "origin").unwrap();
            config
                .set_str("branch.main.merge", "refs/heads/main")
                .unwrap();
        }
        let app_data = private.path().join("app");
        fs::create_dir_all(&app_data).unwrap();
        Self {
            root,
            gitdir,
            gitfile,
            app_data,
            store,
            repo,
            bare,
            url,
            home,
            _private: private,
        }
    }

    fn slot(&self) -> GraphSlot {
        let mut slot = GraphSlot::new(self.store.clone(), self.root.clone());
        let host = PageHost::start_for_tests(&self.store, &self.app_data).unwrap();
        *slot.host.get_mut().unwrap() = PageHostSlot::Running(host);
        slot
    }

    fn edit(&self, path: &str, text: &str) {
        fs::write(self.root.join(path), text).unwrap();
        self.store.refresh(Depth::Bytes).unwrap();
    }

    fn remote_tip(&self) -> Option<Oid> {
        self.bare
            .find_reference("refs/heads/main")
            .ok()
            .and_then(|r| r.target())
    }

    /// Another device clones, edits `pages/a.md`, commits and pushes.
    fn peer_pushes(&self, text: &str) -> Oid {
        let dir = self._private.path().join("peer");
        let peer = Repository::clone(&self.url, &dir).unwrap();
        // The peer is a plain checkout like the graph: a host-wide
        // `core.autocrlf` (Windows runners set it) must not make its push refuse.
        peer.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        fs::write(dir.join("pages/a.md"), text).unwrap();
        let oid = commit_all(&peer, "from the laptop");
        assert_eq!(operation().push_upstream(&dir), Net::UpToDate);
        oid
    }

    /// The layout is as the user made it: the graph's `.git` is still the
    /// same file (no `.git` folder, no rewrite), the git directory has no
    /// `core.worktree` and holds only git's own entries with no lock or index
    /// copy left, nothing was written beside it, and the home folder gained
    /// nothing.
    fn assert_layout_kept(&self, layout: Layout) {
        let marker = fs::symlink_metadata(self.root.join(".git")).unwrap();
        assert!(marker.is_file(), "{layout:?}: the graph got a .git folder");
        assert_eq!(fs::read(self.root.join(".git")).unwrap(), self.gitfile);
        let reopened = Repository::open(&self.gitdir).unwrap();
        assert!(reopened
            .config()
            .unwrap()
            .get_path("core.worktree")
            .is_err());
        const GITS_OWN: &[&str] = &[
            "HEAD",
            "config",
            "description",
            "index",
            "objects",
            "refs",
            "logs",
            "hooks",
            "info",
            "branches",
            "packed-refs",
            "FETCH_HEAD",
            "ORIG_HEAD",
            "COMMIT_EDITMSG",
        ];
        for entry in fs::read_dir(&self.gitdir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                GITS_OWN.contains(&name.as_str()),
                "{layout:?}: {name} in the separate git directory"
            );
        }
        let mut dirs = vec![self.gitdir.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().unwrap().is_dir() {
                    dirs.push(entry.path());
                } else {
                    assert!(
                        !name.ends_with(".lock") && !name.starts_with("tine-"),
                        "{layout:?}: {} left in the git directory",
                        entry.path().display()
                    );
                }
            }
        }
        let beside: Vec<_> = fs::read_dir(self.gitdir.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(beside, [".git"], "{layout:?}: written beside the git dir");
        let mut home: Vec<_> = fs::read_dir(self.home.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        home.sort();
        assert_eq!(home, [".logseq", "logseq"], "{layout:?}");
    }
}

fn configure_libgit2_for_tests() {
    crate::owner::git_run::configure_libgit2();
}

const BOTH: [&str; 2] = [".syncthing.*", "logseq/.tine-*"];

fn separate_git_dir_commits_and_syncs(layout: Layout) {
    let g = Graph::new(layout);
    // git's answer: the work tree is the folder holding the `.git` file.
    let found = crate::owner::git_run::discover(&g.root).unwrap();
    assert_eq!(
        fs::canonicalize(found.workdir().unwrap()).unwrap(),
        fs::canonicalize(&g.root).unwrap(),
        "{layout:?}: libgit2 path {}, graph {}",
        found.path().display(),
        g.root.display()
    );
    drop(found);
    // Open: Settings offers commits and sync, no refusal.
    let status = git_status(
        &g.root,
        GitSettings {
            policy: Policy::SyncAutomatically,
            interval: Duration::from_secs(600),
        },
    );
    assert!(status.repository, "{layout:?}: {status:?}");
    assert_eq!(status.unusable, None, "{layout:?}");
    assert_eq!(status.sync_unusable, None, "{layout:?}");
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.remote.is_some(), "{layout:?}: {status:?}");

    // The .gitignore step writes the graph root's .gitignore (never the git
    // directory's info/exclude).
    assert_eq!(
        prepare_git_ignore(&g.store, &g.root, Some(&g.app_data)).unwrap(),
        BOTH,
        "{layout:?}"
    );
    assert_eq!(
        fs::read_to_string(g.root.join(".gitignore")).unwrap(),
        ".syncthing.*\nlogseq/.tine-*\n"
    );
    assert!(g
        .repo
        .status_should_ignore(Path::new("pages/.syncthing.a.md.tmp"))
        .unwrap());

    // The first push records the upstream, as setup would.
    assert_eq!(operation().push_upstream(&g.root), Net::UpToDate);
    assert_eq!(operation().fetch(&g.root), Net::UpToDate);

    let slot = Arc::new(g.slot());
    start_worker(
        &slot,
        GitSettings {
            policy: Policy::SyncAutomatically,
            interval: Duration::from_secs(600),
        },
        Arc::new(NoCredentials),
    );
    wait_for("the opening cycle", || {
        let sync = last_attempt(&g.root).sync;
        !sync.running && sync.at.is_some()
    });

    // Automatic commit (and its push), without waiting for the cadence: the
    // app goes to the background.
    let before = head(&g.repo);
    g.edit("pages/b.md", "- typed, then the app went away\n");
    backgrounded_at(&g.root);
    wait_for("the automatic commit and push", || {
        head(&g.repo) != before && g.remote_tip() == Some(head(&g.repo))
    });
    let tip = g.repo.find_commit(head(&g.repo)).unwrap();
    assert_eq!(tip.message().ok(), Some(MESSAGE));
    assert!(tip
        .tree()
        .unwrap()
        .get_path(Path::new(".gitignore"))
        .is_ok());
    assert!(tip
        .tree()
        .unwrap()
        .get_path(Path::new("pages/.syncthing.a.md.tmp"))
        .is_err());
    foregrounded_at(&g.root);
    wait_for("the foreground cycle", || {
        !last_attempt(&g.root).sync.running
    });

    // Commit and sync on demand: "Sync now" commits, then pushes.
    set_settings(
        &g.root,
        GitSettings {
            policy: Policy::SyncOnDemand,
            interval: Duration::from_secs(600),
        },
    );
    let before = head(&g.repo);
    g.edit("pages/a.md", "- one\n- edited here\n");
    assert!(request_sync(&g.root));
    wait_for("the commit and push on demand", || {
        head(&g.repo) != before && g.remote_tip() == Some(head(&g.repo))
    });
    wait_for("the cycle's end", || !last_attempt(&g.root).sync.running);

    // Sync on demand pulls what another device pushed, through the page host.
    let theirs = g.peer_pushes("- one\n- edited here\n- from the laptop\n");
    assert!(request_sync(&g.root));
    wait_for("the pull on demand", || head(&g.repo) == theirs);
    wait_for("the cycle's end", || !last_attempt(&g.root).sync.running);
    assert_eq!(
        last_attempt(&g.root).sync.code.as_deref(),
        Some("pulled"),
        "{layout:?}"
    );
    assert_eq!(
        fs::read_to_string(g.root.join("pages/a.md")).unwrap(),
        "- one\n- edited here\n- from the laptop\n"
    );
    slot.background_cancelled.store(true, Ordering::Release);
    drop(slot);
    g.assert_layout_kept(layout);
}

#[test]
#[cfg_attr(
    any(target_os = "android", target_os = "ios"),
    ignore = "needs the git command line"
)]
fn gh872_a_graph_made_by_git_init_separate_git_dir_commits_and_syncs() {
    separate_git_dir_commits_and_syncs(Layout::GitCli);
}

#[test]
fn gh872_a_graph_with_an_absolute_gitdir_file_commits_and_syncs() {
    separate_git_dir_commits_and_syncs(Layout::Absolute);
}

#[test]
fn gh872_a_graph_with_a_relative_gitdir_file_commits_and_syncs() {
    separate_git_dir_commits_and_syncs(Layout::Relative);
}

/// A graph in a subfolder of a work tree whose git directory is separate:
/// commits only the graph, under its prefix; sync still refuses (§1).
#[test]
fn gh872_a_graph_inside_a_separate_dir_work_tree_commits_only_itself() {
    let g = Graph::new(Layout::Absolute);
    let nested = g.root.join("notes");
    fs::create_dir_all(nested.join("pages")).unwrap();
    fs::write(nested.join("pages/n.md"), "- nested\n").unwrap();
    fs::write(g.root.join("pages/b.md"), "- outside the nested graph\n").unwrap();
    assert!(matches!(
        commit_now(&nested, MESSAGE),
        Ok(Outcome::Committed(_))
    ));
    let tree = g.repo.find_commit(head(&g.repo)).unwrap().tree().unwrap();
    assert!(tree.get_path(Path::new("notes/pages/n.md")).is_ok());
    let b = tree.get_path(Path::new("pages/b.md")).unwrap();
    assert_eq!(
        g.repo.find_blob(b.id()).unwrap().content(),
        b"- see [[a]]\n",
        "a change outside the nested graph is not committed"
    );
    let status = git_status(&nested, GitSettings::default());
    assert_eq!(status.unusable, None);
    assert!(matches!(
        status.sync_unusable.as_ref(),
        Some((_, why)) if why.contains("work tree root")
    ));
    assert!(fs::symlink_metadata(g.root.join(".git")).unwrap().is_file());
}

/// The cases that really are unsafe keep refusing, each by name, and the
/// refusal changes nothing: a `.git` file naming no repository (Logseq's git
/// folder was deleted or never synced to this device), a malformed `.git`
/// file, and a `core.worktree` that puts the work tree elsewhere.
#[test]
fn gh872_a_broken_separate_git_dir_refuses_by_name_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/a.md"), "- a\n").unwrap();
    for (gitfile, says) in [
        (
            format!("gitdir: {}\n", dir.path().join("gone/.git").display()),
            "not a git repository",
        ),
        ("not a gitfile\n".to_owned(), "malformed"),
    ] {
        fs::write(root.join(".git"), &gitfile).unwrap();
        let status = git_status(&root, GitSettings::default());
        assert!(
            matches!(&status.unusable, Some((_, why)) if why.contains(says)),
            "{gitfile:?}: {status:?}"
        );
        assert!(commit_now(&root, MESSAGE)
            .unwrap_err()
            .message()
            .contains(says));
        let store = Store::open(&root, Default::default()).unwrap().0;
        assert!(prepare_git_ignore(&store, &root, Some(&dir.path().join("app"))).is_err());
        assert_eq!(fs::read_to_string(root.join(".git")).unwrap(), gitfile);
        assert!(!root.join(".gitignore").exists());
        assert!(!dir.path().join("gone").exists());
    }

    // core.worktree elsewhere: git's work tree is not the graph.
    let g = Graph::new(Layout::Absolute);
    let elsewhere = g.home.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    g.repo
        .config()
        .unwrap()
        .set_str("core.worktree", &elsewhere.to_string_lossy())
        .unwrap();
    let status = git_status(&g.root, GitSettings::default());
    assert!(
        matches!(&status.unusable, Some((_, why)) if why.contains("not inside its repository's work tree")),
        "{status:?}"
    );
    let index = fs::read(g.gitdir.join("index")).unwrap();
    let tip = head(&g.repo);
    assert!(commit_now(&g.root, MESSAGE).is_err());
    assert_eq!(fs::read(g.gitdir.join("index")).unwrap(), index);
    assert_eq!(head(&g.repo), tip);
}
