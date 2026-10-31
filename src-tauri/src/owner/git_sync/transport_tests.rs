use super::*;
use git2::{Repository, Signature};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

fn tempdir() -> tempfile::TempDir {
    // Self-deleting, so a panic leaves nothing behind (testTempDirs guard).
    tempfile::Builder::new()
        .prefix("tine-clone-cleanup-")
        .tempdir_in(std::env::temp_dir())
        .unwrap()
}

fn init(path: &Path) -> Repository {
    let mut options = git2::RepositoryInitOptions::new();
    options.initial_head("main");
    let repo = Repository::init_opts(path, &options).unwrap();
    repo.config()
        .unwrap()
        .set_bool("core.autocrlf", false)
        .unwrap();
    repo.config()
        .unwrap()
        .set_bool("core.precomposeunicode", true)
        .unwrap();
    repo
}

fn commit(repo: &Repository, text: &str) -> Oid {
    let root = repo.workdir().unwrap();
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/a.md"), text).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("pages/a.md")).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let parent = repo.head().ok().map(|h| h.peel_to_commit().unwrap());
    let parents: Vec<_> = parent.iter().collect();
    let signature = Signature::now("Transport test", "transport@example.invalid").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, text, &tree, &parents)
        .unwrap()
}

fn track(repo: &Repository, url: &str) {
    if repo.find_remote("origin").is_ok() {
        repo.remote_set_url("origin", url).unwrap();
    } else {
        repo.remote("origin", url).unwrap();
    }
    let mut config = repo.config().unwrap();
    config.set_str("branch.main.remote", "origin").unwrap();
    config
        .set_str("branch.main.merge", "refs/heads/main")
        .unwrap();
}

fn operation() -> Operation {
    Operation::new(
        Arc::new(NoCredentials),
        Arc::new(AtomicBool::new(false)),
        &TransportOptions::default(),
    )
    .unwrap()
}

#[test]
fn clone_default_branch_sets_upstream_and_refuses_nonempty_target() {
    let f = Fixture::new();
    let root = f._dir.path().join("cloned");
    assert_eq!(operation().clone_graph(&f.url, &root), Outcome::UpToDate);
    let repo = Repository::open(&root).unwrap();
    assert_eq!(repo.head().unwrap().shorthand().unwrap(), "main");
    assert_eq!(
        repo.branch_upstream_name("refs/heads/main")
            .unwrap()
            .as_str()
            .unwrap(),
        "refs/remotes/origin/main"
    );
    assert_eq!(repo.head().unwrap().target(), Some(f.base));
    assert_eq!(fs::read(root.join("pages/a.md")).unwrap(), b"- base\n");
    assert_eq!(operation().fetch(&root), Outcome::UpToDate);
    assert!(
        matches!(operation().clone_graph(&f.url, &root), Outcome::Refused(m) if m.contains("not empty"))
    );
    assert_eq!(fs::read(root.join("pages/a.md")).unwrap(), b"- base\n");
    crate::git_setup::tests::report_footprint("clone", &root);
}

#[test]
fn clone_uses_the_remote_default_branch_even_when_it_is_not_main() {
    let f = Fixture::new();
    f.bare
        .reference("refs/heads/notes", f.base, false, "fixture")
        .unwrap();
    f.bare.set_head("refs/heads/notes").unwrap();
    let root = f._dir.path().join("cloned");
    assert_eq!(operation().clone_graph(&f.url, &root), Outcome::UpToDate);
    let repo = Repository::open(&root).unwrap();
    assert_eq!(repo.head().unwrap().shorthand().unwrap(), "notes");
    assert_eq!(
        repo.branch_upstream_name("refs/heads/notes")
            .unwrap()
            .as_str()
            .unwrap(),
        "refs/remotes/origin/notes"
    );
    assert_eq!(operation().fetch(&root), Outcome::UpToDate);
}

#[test]
fn clone_refuses_a_remote_without_commits_and_points_to_initial_setup() {
    let f = Fixture::new();
    f.bare
        .find_reference("refs/heads/main")
        .unwrap()
        .delete()
        .unwrap();
    let root = f._dir.path().join("cloned");
    assert!(
        matches!(operation().clone_graph(&f.url, &root), Outcome::Refused(m) if m.contains("no commits") && m.contains("Make this graph a git repository"))
    );
    assert!(!root.exists(), "a refused clone must remove its new folder");
}

#[test]
fn clone_failure_preserves_a_preexisting_empty_folder_and_allows_retry() {
    let f = Fixture::new();
    let root = f._dir.path().join("cloned");
    fs::create_dir(&root).unwrap();
    let missing = reqwest::Url::from_file_path(f._dir.path().join("missing.git"))
        .unwrap()
        .to_string();
    assert_ne!(operation().clone_graph(&missing, &root), Outcome::UpToDate);
    assert!(root.is_dir());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    assert_eq!(operation().clone_graph(&f.url, &root), Outcome::UpToDate);
}

#[test]
fn clone_missing_default_branch_removes_its_new_folder() {
    let f = Fixture::new();
    f.bare.set_head("refs/heads/missing").unwrap();
    let root = f._dir.path().join("cloned");
    assert!(matches!(
        operation().clone_graph(&f.url, &root),
        Outcome::Refused(_)
    ));
    assert!(!root.exists());
}

fn initial_graph(f: &Fixture, url: &str) -> PathBuf {
    let root = f._dir.path().join("initial-graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/a.md"), b"- first graph\r\n").unwrap();
    crate::git_setup::tests::initialize_fixture(&root, &f._dir.path().join("device"), url);
    root
}

#[test]
fn initial_setup_pushes_to_empty_remote_and_sets_upstream_with_resumable_confirmation() {
    let f = Fixture::new();
    f.bare
        .find_reference("refs/heads/main")
        .unwrap()
        .delete()
        .unwrap();
    let root = initial_graph(&f, &f.url);
    assert!(initial_push_pending(&root));
    assert_eq!(operation().push_initial(&root), Outcome::UpToDate);
    let repo = Repository::open(&root).unwrap();
    let tip = repo.head().unwrap().target().unwrap();
    assert_eq!(f.bare.head().unwrap().target(), Some(tip));
    assert_eq!(repo.refname_to_id("refs/remotes/origin/main").unwrap(), tip);
    assert!(!initial_push_pending(&root));
    assert_eq!(operation().fetch(&root), Outcome::UpToDate);
    assert_eq!(
        fs::read(root.join("pages/a.md")).unwrap(),
        b"- first graph\r\n"
    );
    crate::git_setup::tests::report_footprint("init+empty remote", &root);
    // A process interrupted after server acceptance but before local tracking
    // publication can confirm precisely that commit, without another commit.
    repo.find_reference("refs/remotes/origin/main")
        .unwrap()
        .delete()
        .unwrap();
    assert!(initial_push_pending(&root));
    assert_eq!(operation().push_initial(&root), Outcome::UpToDate);
    assert!(!initial_push_pending(&root));
}

#[test]
fn initial_setup_refuses_a_nonempty_remote_and_can_keep_only_local_history() {
    let f = Fixture::new();
    let root = initial_graph(&f, &f.url);
    assert!(
        matches!(operation().push_initial(&root), Outcome::Refused(m) if m.contains("not empty"))
    );
    assert_eq!(f.bare.head().unwrap().target(), Some(f.base));
    assert!(initial_push_pending(&root));
    configure_initial_remote(&root, "").unwrap();
    assert!(!initial_push_pending(&root));
    assert_eq!(
        fs::read(root.join("pages/a.md")).unwrap(),
        b"- first graph\r\n"
    );
}

#[test]
fn clone_refuses_folding_tree_before_checkout_including_directories_and_nfc() {
    for names in [
        ["A.md", "a.md"],
        ["É.md", "E\u{301}.md"],
        ["Folder/a.md", "folder/b.md"],
    ] {
        let f = Fixture::new();
        // Build the colliding tree in memory, so the fixture also runs on a
        // folding host. No reliance on the host representing both names.
        let blob = f.repo.blob(b"- name\n").unwrap();
        let mut builder = git2::build::TreeUpdateBuilder::new();
        for name in names {
            builder.upsert(name, blob, git2::FileMode::Blob);
        }
        let empty = f.repo.treebuilder(None).unwrap().write().unwrap();
        let tree = f
            .repo
            .find_tree(
                builder
                    .create_updated(&f.repo, &f.repo.find_tree(empty).unwrap())
                    .unwrap(),
            )
            .unwrap();
        let signature = Signature::now("Fixture", "fixture@example.invalid").unwrap();
        let oid = f
            .repo
            .commit(None, &signature, &signature, "colliding names", &tree, &[])
            .unwrap();
        f.repo
            .reference("refs/heads/main", oid, true, "fixture")
            .unwrap();
        // Publish objects to a new bare remote through libgit2's local fetch.
        let mut remote = f
            .bare
            .remote_anonymous(&reqwest::Url::from_file_path(&f.root).unwrap().to_string())
            .unwrap();
        remote
            .fetch(&["+refs/heads/main:refs/heads/main"], None, None)
            .unwrap();
        let root = f._dir.path().join("cloned");
        let result = operation().clone_on_volume(&f.url, &root, Some(true));
        assert!(
            matches!(&result, Outcome::Refused(m) if m.contains("Unicode normalization")),
            "{result:?}"
        );
        assert!(!root.exists(), "a refused clone must remove its new folder");
        // Repair the remote, then retry the exact destination without manual cleanup.
        f.bare
            .reference("refs/heads/main", f.base, true, "repair fixture")
            .unwrap();
        assert_eq!(
            operation().clone_on_volume(&f.url, &root, Some(true)),
            Outcome::UpToDate
        );
        assert_eq!(fs::read(root.join("pages/a.md")).unwrap(), b"- base\n");
    }
}

#[test]
fn clone_url_names_are_literal_components_and_https_credentials_are_refused() {
    assert_eq!(
        super::clone::repository_name("https://example.invalid/me/notes.git").unwrap(),
        "notes"
    );
    for url in [
        "https://u:secret@example.invalid/a.git",
        "https://example.invalid/a.git?token=secret",
        "https://example.invalid/a%2fb.git",
        "ssh://example.invalid/a.git",
        "https://example.invalid/",
    ] {
        assert!(super::clone::repository_name(url).is_err(), "{url}");
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: Repository,
    bare: Repository,
    base: Oid,
    url: String,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempdir();
        let root = dir.path().join("graph");
        let repo = init(&root);
        let base = commit(&repo, "- base\n");
        let remote_path = dir.path().join("remote.git");
        let bare = Repository::init_bare(&remote_path).unwrap();
        bare.set_head("refs/heads/main").unwrap();
        bare.config()
            .unwrap()
            .set_bool("http.receivepack", true)
            .unwrap();
        let url = reqwest::Url::from_file_path(&remote_path)
            .unwrap()
            .to_string();
        track(&repo, &url);
        assert_eq!(operation().push_upstream(&root), Outcome::UpToDate);
        Self {
            _dir: dir,
            root,
            repo,
            bare,
            base,
            url,
        }
    }

    fn peer(&self) -> Repository {
        let path = self
            ._dir
            .path()
            .join(format!("peer-{}", uuid::Uuid::new_v4()));
        let repo = Repository::clone(&self.url, path).unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.precomposeunicode", true)
            .unwrap();
        repo
    }

    fn remote_commit(&self, text: &str) -> Oid {
        let peer = self.peer();
        let oid = commit(&peer, text);
        assert_eq!(
            operation().push_upstream(peer.workdir().unwrap()),
            Outcome::UpToDate
        );
        oid
    }

    fn snapshot(&self) -> Vec<Vec<u8>> {
        [
            self.repo.path().join("HEAD"),
            self.repo.path().join("index"),
            self.repo.path().join("refs/heads/main"),
            self.root.join("pages/a.md"),
        ]
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect()
    }
}

#[test]
fn file_fetch_updates_only_tracking_ref_and_preserves_dirty_worktree_and_index() {
    let f = Fixture::new();
    assert_eq!(operation().fetch(&f.root), Outcome::UpToDate);
    let upstream = f.remote_commit("- upstream\n");
    fs::write(f.root.join("pages/a.md"), "- local draft\n").unwrap();
    fs::write(f.repo.path().join("FETCH_HEAD"), "leave this alone\n").unwrap();
    let before = f.snapshot();
    assert_eq!(
        operation().fetch(&f.root),
        Outcome::Fetched {
            head: f.base,
            upstream,
            tracking_ref: "refs/remotes/origin/main".into(),
            ahead: 0,
            behind: 1,
        }
    );
    assert_eq!(f.snapshot(), before);
    assert_eq!(
        fs::read_to_string(f.repo.path().join("FETCH_HEAD")).unwrap(),
        "leave this alone\n"
    );
    assert_eq!(
        f.repo
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .target(),
        Some(upstream)
    );
    assert!(!f.repo.path().join("index.lock").exists());
}

#[test]
fn file_push_and_fetch_classify_ahead_and_diverged_without_force() {
    let f = Fixture::new();
    assert_eq!(operation().fetch(&f.root), Outcome::UpToDate);
    let local = commit(&f.repo, "- local\n");
    assert!(matches!(
        operation().fetch(&f.root),
        Outcome::Fetched {
            ahead: 1,
            behind: 0,
            ..
        }
    ));
    let upstream = f.remote_commit("- remote\n");
    let before = f.snapshot();
    assert_eq!(operation().push_upstream(&f.root), Outcome::Diverged);
    assert_eq!(operation().fetch(&f.root), Outcome::Diverged);
    assert_eq!(f.snapshot(), before);
    assert_eq!(f.repo.head().unwrap().target(), Some(local));
    assert_eq!(
        f.bare.find_reference("refs/heads/main").unwrap().target(),
        Some(upstream)
    );
}

#[test]
fn fetch_ignores_dangerous_configured_refspecs_and_never_fetches_tags() {
    let f = Fixture::new();
    let upstream = f.remote_commit("- remote\n");
    f.bare
        .tag_lightweight(
            "test-tag",
            &f.bare.find_object(upstream, None).unwrap(),
            false,
        )
        .unwrap();
    f.repo
        .config()
        .unwrap()
        .set_str("remote.origin.fetch", "+refs/heads/*:refs/heads/*")
        .unwrap();
    // An unsafe upstream mapping is refused before any transfer.
    let before = f.snapshot();
    assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
    assert_eq!(f.snapshot(), before);
    f.repo
        .config()
        .unwrap()
        .set_str("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
        .unwrap();
    f.repo
        .config()
        .unwrap()
        .set_str("remote.origin.tagOpt", "--tags")
        .unwrap();
    f.repo
        .config()
        .unwrap()
        .set_bool("remote.origin.prune", true)
        .unwrap();
    f.repo
        .reference("refs/remotes/origin/keep", f.base, false, "test")
        .unwrap();
    assert!(
        matches!(operation().fetch(&f.root), Outcome::Fetched { upstream: oid, .. } if oid == upstream)
    );
    assert!(f.repo.find_reference("refs/tags/test-tag").is_err());
    assert!(f.repo.find_reference("refs/remotes/origin/keep").is_ok());
    assert_eq!(f.snapshot(), before);
}

#[test]
fn sync_refuses_unsupported_repositories_and_conversion_and_busy_locks() {
    let f = Fixture::new();
    fs::write(f.repo.path().join("index.lock"), "other git command").unwrap();
    assert_eq!(operation().fetch(&f.root), Outcome::Busy);
    assert_eq!(operation().push_upstream(&f.root), Outcome::Busy);
    fs::remove_file(f.repo.path().join("index.lock")).unwrap();
    assert!(matches!(
        operation().fetch(&f.root.join("pages")),
        Outcome::Refused(_)
    ));
    assert!(matches!(
        operation().fetch(f.bare.path()),
        Outcome::Refused(_)
    ));
    for value in ["true", "input"] {
        f.repo
            .config()
            .unwrap()
            .set_str("core.autocrlf", value)
            .unwrap();
        assert!(
            matches!(operation().fetch(&f.root), Outcome::Refused(m) if m.contains("conversion"))
        );
    }
    f.repo
        .config()
        .unwrap()
        .set_bool("core.autocrlf", false)
        .unwrap();
    for attr in [
        "text",
        "eol=lf",
        "filter=encrypt",
        "ident",
        "working-tree-encoding=UTF-16",
    ] {
        fs::write(
            f.root.join(".gitattributes"),
            format!("pages/a.md {attr}\n"),
        )
        .unwrap();
        assert!(
            matches!(operation().fetch(&f.root), Outcome::Refused(m) if m.contains("conversion")),
            "{attr}"
        );
    }
    fs::remove_file(f.root.join(".gitattributes")).unwrap();
    f.repo
        .config()
        .unwrap()
        .set_bool("core.sparseCheckout", true)
        .unwrap();
    assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
    f.repo
        .config()
        .unwrap()
        .set_bool("core.sparseCheckout", false)
        .unwrap();
    f.repo.set_head_detached(f.base).unwrap();
    assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
    f.repo.set_head("refs/heads/main").unwrap();
    fs::write(f.repo.path().join("MERGE_HEAD"), format!("{}\n", f.base)).unwrap();
    assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
}

#[test]
fn unsupported_remote_configs_are_refused_without_contacting_them() {
    let f = Fixture::new();
    let before = f.snapshot();
    for url in [
        "ssh://example.invalid/graph",
        "http://example.invalid/graph",
        "https://user:secret@example.invalid/graph",
        "https://example.invalid/graph?token=secret",
    ] {
        track(&f.repo, url);
        assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
        assert!(matches!(
            operation().push_upstream(&f.root),
            Outcome::Refused(_)
        ));
    }
    track(&f.repo, &f.url);
    f.repo
        .config()
        .unwrap()
        .set_str("branch.main.remote", ".")
        .unwrap();
    assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
    track(&f.repo, &f.url);
    f.repo
        .config()
        .unwrap()
        .set_bool("remote.origin.mirror", true)
        .unwrap();
    assert!(matches!(
        operation().push_upstream(&f.root),
        Outcome::Refused(_)
    ));
    assert_eq!(f.snapshot(), before);
}

#[test]
fn url_rewrites_cannot_bypass_the_validated_fetch_or_push_endpoint() {
    for setting in ["insteadOf", "pushInsteadOf"] {
        let f = Fixture::new();
        let sink_path = f._dir.path().join("sink.git");
        let sink = Repository::init_bare(&sink_path).unwrap();
        let sink_url = reqwest::Url::from_file_path(&sink_path)
            .unwrap()
            .to_string();
        f.repo
            .config()
            .unwrap()
            .set_str(&format!("url.{sink_url}.{setting}"), &f.url)
            .unwrap();
        let before = f.snapshot();
        let outcome = operation().push_upstream(&f.root);
        assert!(
            matches!(outcome, Outcome::Refused(_)),
            "{setting}: {outcome:?}"
        );
        assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
        assert!(sink.references().unwrap().next().is_none());
        assert_eq!(f.snapshot(), before);
    }
}

#[test]
fn push_uses_tracked_branch_name_and_ignores_extra_push_refspecs() {
    let f = Fixture::new();
    f.repo
        .config()
        .unwrap()
        .set_str("branch.main.merge", "refs/heads/different")
        .unwrap();
    f.repo
        .config()
        .unwrap()
        .set_str("remote.origin.push", "+refs/heads/main:refs/heads/extra")
        .unwrap();
    assert_eq!(operation().push_upstream(&f.root), Outcome::UpToDate);
    assert_eq!(
        f.bare
            .find_reference("refs/heads/different")
            .unwrap()
            .target(),
        Some(f.base)
    );
    assert!(f.bare.find_reference("refs/heads/extra").is_err());
}

struct TestCredentials {
    calls: AtomicUsize,
    secret: Option<String>,
    repository: String,
}
impl Credentials for TestCredentials {
    fn token(&self, repository: &str) -> Option<Token> {
        assert_eq!(repository, self.repository);
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.secret.as_ref().map(|secret| Token {
            username: "token-user".into(),
            secret: secret.clone(),
        })
    }
}

fn python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}
fn harness() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/owner/git_sync/fixture.py")
}
fn certs() -> tempfile::TempDir {
    // Cache bytes, not a static TempDir (statics are not dropped at exit).
    // Each caller's certificate directory is self-deleting like its graph fixture.
    static CERTS: OnceLock<Vec<(std::ffi::OsString, Vec<u8>)>> = OnceLock::new();
    let files = CERTS.get_or_init(|| {
        let dir = tempdir();
        let output = Command::new(python())
            .arg(harness())
            .arg("--certs")
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "test CA generator: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), fs::read(entry.path()).unwrap())
            })
            .collect()
    });
    let dir = tempdir();
    for (name, bytes) in files {
        fs::write(dir.path().join(name), bytes).unwrap();
    }
    dir
}

struct Server {
    child: Child,
    url: String,
    log: PathBuf,
    _dir: tempfile::TempDir,
}
impl Server {
    fn start(f: &Fixture, mode: &str, destination: &str) -> Self {
        let dir = tempdir();
        let certificates = certs();
        let log = dir.path().join("requests.jsonl");
        let mut child = Command::new(python())
            .arg(harness())
            .arg(f.bare.path().parent().unwrap())
            .arg(certificates.path())
            .arg(&log)
            .arg(mode)
            .arg(destination)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut ready = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        let port: u16 = ready
            .trim()
            .strip_prefix("READY ")
            .expect("TLS fixture ready")
            .parse()
            .unwrap();
        Self {
            child,
            url: format!("https://127.0.0.1:{port}"),
            log,
            _dir: dir,
        }
    }

    fn remote(&self) -> String {
        format!("{}/remote.git", self.url)
    }
    fn requests(&self) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
    fn operation(&self, secret: Option<&str>) -> (Operation, Arc<TestCredentials>) {
        self.operation_at(
            secret,
            self.remote(),
            &TransportOptions::default(),
            Arc::new(AtomicBool::new(false)),
        )
    }
    fn operation_at(
        &self,
        secret: Option<&str>,
        repository: String,
        options: &TransportOptions,
        cancel: Arc<AtomicBool>,
    ) -> (Operation, Arc<TestCredentials>) {
        let credentials = Arc::new(TestCredentials {
            calls: AtomicUsize::new(0),
            secret: secret.map(str::to_owned),
            repository,
        });
        let options = TransportOptions {
            connect_timeout: options.connect_timeout,
            read_timeout: options.read_timeout,
            request_timeout: options.request_timeout,
            max_request_body: options.max_request_body,
            test_root_pem: Some(fs::read(certs().path().join("ca.pem")).unwrap()),
        };
        (
            Operation::new(credentials.clone(), cancel, &options).unwrap(),
            credentials,
        )
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn https_clone_uses_the_existing_private_token_store_after_auth_required() {
    let f = Fixture::new();
    let server = Server::start(&f, "auth", "");
    let root = f._dir.path().join("cloned");
    assert_eq!(
        server
            .operation(None)
            .0
            .clone_graph(&server.remote(), &root),
        Outcome::AuthRequired
    );
    assert!(
        !root.exists(),
        "authentication retry must start with a clean folder"
    );
    let app_data = f._dir.path().join("private-device-data");
    let tokens = credentials::Tokens::new(&app_data);
    tokens
        .sign_in(&server.url, "token-user", "fixture-token")
        .unwrap();
    let options = TransportOptions {
        test_root_pem: Some(fs::read(certs().path().join("ca.pem")).unwrap()),
        ..TransportOptions::default()
    };
    let op = Operation::new(Arc::new(tokens), Arc::new(AtomicBool::new(false)), &options).unwrap();
    assert_eq!(op.clone_graph(&server.remote(), &root), Outcome::UpToDate);
    assert_eq!(fs::read(root.join("pages/a.md")).unwrap(), b"- base\n");
    let config = fs::read_to_string(root.join(".git/config")).unwrap();
    assert!(!config.contains("fixture-token"));
    assert!(!root.join(credentials::FILE).exists());
}

#[test]
fn https_initial_setup_uses_existing_token_store_and_retries_without_another_commit() {
    let f = Fixture::new();
    f.bare
        .find_reference("refs/heads/main")
        .unwrap()
        .delete()
        .unwrap();
    let server = Server::start(&f, "auth", "");
    let root = initial_graph(&f, &server.remote());
    let repo = Repository::open(&root).unwrap();
    let tip = repo.head().unwrap().target();
    assert_eq!(
        server.operation(None).0.push_initial(&root),
        Outcome::AuthRequired
    );
    assert!(initial_push_pending(&root));
    let tokens = credentials::Tokens::new(f._dir.path().join("device-token"));
    tokens
        .sign_in(&server.url, "token-user", "fixture-token")
        .unwrap();
    let options = TransportOptions {
        test_root_pem: Some(fs::read(certs().path().join("ca.pem")).unwrap()),
        ..TransportOptions::default()
    };
    let op = Operation::new(Arc::new(tokens), Arc::new(AtomicBool::new(false)), &options).unwrap();
    assert_eq!(op.push_initial(&root), Outcome::UpToDate);
    assert_eq!(repo.head().unwrap().target(), tip);
    assert_eq!(f.bare.head().unwrap().target(), tip);
    assert!(!initial_push_pending(&root));
    assert!(!fs::read_to_string(root.join(".git/config"))
        .unwrap()
        .contains("fixture-token"));
}

#[test]
fn https_fetch_and_push_retry_401_once_and_keep_graph_bytes_unchanged() {
    let f = Fixture::new();
    let upstream = f.remote_commit("- remote over TLS\n");
    let server = Server::start(&f, "auth", "");
    track(&f.repo, &server.remote());
    let before = f.snapshot();
    let (op, provider) = server.operation(Some("fixture-token"));
    let outcome = op.fetch(&f.root);
    assert!(
        matches!(outcome, Outcome::Fetched { upstream: oid, behind: 1, .. } if oid == upstream),
        "{outcome:?}"
    );
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    assert_eq!(f.snapshot(), before);
    let requests = server.requests();
    assert_eq!(requests[0]["status"], 401);
    assert_eq!(requests[0]["auth"], false);
    assert_eq!(requests[1]["status"], 200);
    assert_eq!(requests[1]["auth"], true);
    assert!(requests
        .iter()
        .any(|r| r["method"] == "POST" && r["auth"] == true));
    // Independent peer is at the remote tip; push a fast-forward via HTTPS.
    let peer = f.peer();
    let pushed = commit(&peer, "- pushed over TLS\n");
    track(&peer, &server.remote());
    assert_eq!(
        server
            .operation(Some("fixture-token"))
            .0
            .push_upstream(peer.workdir().unwrap()),
        Outcome::UpToDate
    );
    assert_eq!(
        f.bare.find_reference("refs/heads/main").unwrap().target(),
        Some(pushed)
    );
    assert_eq!(
        server
            .operation(Some("fixture-token"))
            .0
            .push_upstream(&f.root),
        Outcome::Diverged
    );
    assert_eq!(
        f.bare.find_reference("refs/heads/main").unwrap().target(),
        Some(pushed)
    );
    let output = Command::new("git")
        .arg("-C")
        .arg(f.bare.path())
        .args(["fsck", "--strict"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// I-25 (the sync worker's idle cycle): when neither side changed, a fetch
/// reads the ref advertisement and asks for no pack.
#[test]
fn an_idle_https_fetch_reads_only_the_advertisement() {
    let f = Fixture::new();
    let server = Server::start(&f, "auth", "");
    track(&f.repo, &server.remote());
    let (op, _) = server.operation(Some("fixture-token"));
    assert_eq!(op.fetch(&f.root), Outcome::UpToDate);
    let before = server.requests().len();
    let (op, _) = server.operation(Some("fixture-token"));
    assert_eq!(op.fetch(&f.root), Outcome::UpToDate);
    let idle: Vec<_> = server.requests().split_off(before);
    assert!(
        idle.iter().all(|r| r["method"] == "GET"),
        "no upload-pack POST: {idle:?}"
    );
    // The advertisement, after the host's 401 challenge (credentials are
    // never sent unasked).
    assert!(idle.len() <= 2, "{idle:?}");
}

#[test]
fn https_missing_or_rejected_token_is_typed_auth_required_with_bounded_retry() {
    for secret in [None, Some("wrong-secret")] {
        let f = Fixture::new();
        let server = Server::start(&f, "auth", "");
        track(&f.repo, &server.remote());
        let (op, provider) = server.operation(secret);
        assert_eq!(op.fetch(&f.root), Outcome::AuthRequired);
        assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            server.requests().len(),
            if secret.is_some() { 2 } else { 1 }
        );
        assert!(!f.repo.path().join("index.lock").exists());
    }
}

#[test]
fn https_redirect_preserves_git_service_and_auth_and_updates_post_base() {
    let f = Fixture::new();
    f.remote_commit("- redirected\n");
    let server = Server::start(&f, "auth", "");
    let remote = format!("{}/old/remote.git", server.url);
    track(&f.repo, &remote);
    let (op, provider) = server.operation_at(
        Some("fixture-token"),
        remote,
        &TransportOptions::default(),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(matches!(
        op.fetch(&f.root),
        Outcome::Fetched { behind: 1, .. }
    ));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    let requests = server.requests();
    assert!(requests
        .iter()
        .any(|r| r["status"] == 301 && r["auth"] == true));
    assert!(requests
        .iter()
        .any(|r| r["path"] == "/remote.git/git-upload-pack" && r["auth"] == true));
}

#[test]
fn https_redirect_to_another_port_never_carries_or_requests_original_token() {
    let f = Fixture::new();
    let target = Server::start(&f, "auth", "");
    let source = Server::start(&f, "auth-redirect", &target.url);
    track(&f.repo, &source.remote());
    let (op, provider) = source.operation(Some("fixture-token"));
    assert_eq!(op.fetch(&f.root), Outcome::AuthRequired);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    assert_eq!(target.requests().len(), 1);
    assert_eq!(target.requests()[0]["auth"], false);
}

#[test]
fn https_downgrade_redirect_loop_bad_content_and_post_redirect_fail_closed() {
    let f = Fixture::new();
    for (mode, destination) in [
        ("redirect", "http://127.0.0.1:1"),
        ("loop", ""),
        ("bad-type", ""),
        ("post-redirect", "https://127.0.0.1:1"),
    ] {
        let server = Server::start(&f, mode, destination);
        track(&f.repo, &server.remote());
        f.remote_commit("- needs a POST\n");
        let outcome = server.operation(None).0.fetch(&f.root);
        assert!(
            matches!(outcome, Outcome::Network(_) | Outcome::Refused(_)),
            "{mode}: {outcome:?}"
        );
        assert!(server.requests().len() <= 6);
    }
}

#[test]
fn https_requires_trusted_certificate() {
    let f = Fixture::new();
    let server = Server::start(&f, "public", "");
    track(&f.repo, &server.remote());
    assert!(matches!(operation().fetch(&f.root), Outcome::Network(_)));
    assert!(server.requests().is_empty(), "TLS must fail before HTTP");
}

#[test]
fn https_post_401_retries_the_buffered_pack_once() {
    let f = Fixture::new();
    let pushed = commit(&f.repo, "- auth on POST\n");
    let server = Server::start(&f, "auth-post", "");
    track(&f.repo, &server.remote());
    let (op, credentials) = server.operation(Some("fixture-token"));
    assert_eq!(op.push_upstream(&f.root), Outcome::UpToDate);
    assert_eq!(credentials.calls.load(Ordering::Relaxed), 1);
    let posts: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r["method"] == "POST")
        .collect();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0]["status"], 401);
    assert_eq!(posts[1]["status"], 200);
    assert_eq!(
        f.bare.find_reference("refs/heads/main").unwrap().target(),
        Some(pushed)
    );
}

#[test]
fn fetch_ref_cas_preserves_a_concurrent_external_fetch() {
    let f = Fixture::new();
    assert_eq!(operation().fetch(&f.root), Outcome::UpToDate);
    let local = commit(&f.repo, "- local\n");
    f.remote_commit("- remote\n");
    let server = Server::start(&f, "hold-fetch", "");
    track(&f.repo, &server.remote());
    let log = server.log.clone();
    let root = f.root.clone();
    let release = f._dir.path().join("release-request");
    let external = std::thread::spawn(move || {
        wait_for_stall(&log);
        let repo = Repository::open(&root).unwrap();
        repo.reference("refs/remotes/origin/main", local, true, "external fetch")
            .unwrap();
        fs::write(release, "release").unwrap();
    });
    let before = f.snapshot();
    let outcome = server.operation(None).0.fetch(&f.root);
    external.join().unwrap();
    assert_eq!(outcome, Outcome::Busy);
    assert_eq!(
        f.repo
            .find_reference("refs/remotes/origin/main")
            .unwrap()
            .target(),
        Some(local)
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn push_refuses_a_checkout_change_during_advertisement() {
    let f = Fixture::new();
    commit(&f.repo, "- local\n");
    f.repo
        .branch("other", &f.repo.find_commit(f.base).unwrap(), false)
        .unwrap();
    let server = Server::start(&f, "hold-ls", "");
    track(&f.repo, &server.remote());
    let log = server.log.clone();
    let root = f.root.clone();
    let release = f._dir.path().join("release-request");
    let external = std::thread::spawn(move || {
        wait_for_stall(&log);
        Repository::open(root)
            .unwrap()
            .set_head("refs/heads/other")
            .unwrap();
        fs::write(release, "release").unwrap();
    });
    let outcome = server.operation(None).0.push_upstream(&f.root);
    external.join().unwrap();
    assert!(matches!(outcome, Outcome::Refused(reason) if reason.contains("branch changed")));
    assert_eq!(
        f.bare.find_reference("refs/heads/main").unwrap().target(),
        Some(f.base)
    );
}

#[test]
fn sync_refuses_symbolic_tracking_refs_and_missing_or_conflicted_indexes() {
    let f = Fixture::new();
    f.repo
        .reference_symbolic("refs/remotes/origin/main", "refs/heads/main", true, "test")
        .unwrap();
    assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
    f.repo
        .find_reference("refs/remotes/origin/main")
        .unwrap()
        .delete()
        .unwrap();
    fs::rename(
        f.repo.path().join("index"),
        f.repo.path().join("saved-index"),
    )
    .unwrap();
    assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
    fs::rename(
        f.repo.path().join("saved-index"),
        f.repo.path().join("index"),
    )
    .unwrap();
    let mut index = f.repo.index().unwrap();
    let mut entry = index.get_path(Path::new("pages/a.md"), 0).unwrap();
    // Index stage 1 denotes a merge conflict; both transports must refuse it.
    entry.flags = (entry.flags & !0x3000) | 0x1000;
    index.add(&entry).unwrap();
    index.write().unwrap();
    assert!(matches!(operation().fetch(&f.root), Outcome::Refused(_)));
    assert!(matches!(
        operation().push_upstream(&f.root),
        Outcome::Refused(_)
    ));
}

fn wait_for_stall(log: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fs::read_to_string(log).unwrap_or_default().contains("102") {
        assert!(Instant::now() < deadline, "fixture never received git POST");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn https_clone_cancellation_and_mid_fetch_failure_remove_the_new_folder() {
    for cancelled in [false, true] {
        let f = Fixture::new();
        let server = Server::start(&f, "stall-fetch", "");
        let root = f._dir.path().join("cloned");
        let cancel = Arc::new(AtomicBool::new(false));
        let options = TransportOptions {
            read_timeout: Duration::from_millis(300),
            ..TransportOptions::default()
        };
        let op = server
            .operation_at(None, server.remote(), &options, cancel.clone())
            .0;
        let canceller = if cancelled {
            let log = server.log.clone();
            Some(std::thread::spawn(move || {
                wait_for_stall(&log);
                cancel.store(true, Ordering::Release);
            }))
        } else {
            None
        };
        let result = op.clone_graph(&server.remote(), &root);
        if let Some(canceller) = canceller {
            canceller.join().unwrap();
            assert_eq!(result, Outcome::Cancelled);
        } else {
            assert!(
                matches!(&result, Outcome::Network(m) if m.contains("timed out")),
                "{result:?}"
            );
        }
        assert!(!root.exists(), "{result:?}");
        assert_eq!(operation().clone_graph(&f.url, &root), Outcome::UpToDate);
    }
}

#[test]
fn https_cancel_aborts_stalled_fetch_and_push_and_releases_locks() {
    for mode in ["stall-fetch", "stall-push"] {
        let f = Fixture::new();
        if mode == "stall-fetch" {
            f.remote_commit("- fetch\n");
        } else {
            commit(&f.repo, "- push\n");
        }
        let server = Server::start(&f, mode, "");
        track(&f.repo, &server.remote());
        let cancel = Arc::new(AtomicBool::new(false));
        let op = server
            .operation_at(
                None,
                server.remote(),
                &TransportOptions::default(),
                cancel.clone(),
            )
            .0;
        let log = server.log.clone();
        let canceller = std::thread::spawn(move || {
            wait_for_stall(&log);
            cancel.store(true, Ordering::Release);
        });
        let started = Instant::now();
        let before = f.snapshot();
        let result = if mode == "stall-fetch" {
            op.fetch(&f.root)
        } else {
            op.push_upstream(&f.root)
        };
        canceller.join().unwrap();
        assert_eq!(result, Outcome::Cancelled, "{mode}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(f.snapshot(), before);
        assert!(!f.repo.path().join("index.lock").exists());
    }
}

#[test]
fn https_timeout_and_buffer_limit_return_typed_errors() {
    let f = Fixture::new();
    f.remote_commit("- fetch timeout\n");
    let server = Server::start(&f, "stall-fetch", "");
    track(&f.repo, &server.remote());
    let options = TransportOptions {
        read_timeout: Duration::from_millis(150),
        ..TransportOptions::default()
    };
    assert!(
        matches!(server.operation_at(None, server.remote(), &options, Arc::new(AtomicBool::new(false))).0.fetch(&f.root), Outcome::Network(m) if m.contains("timed out"))
    );
    let peer = f.peer();
    commit(&peer, "- buffer limit\n");
    track(&peer, &server.remote());
    let options = TransportOptions {
        max_request_body: 1,
        ..TransportOptions::default()
    };
    assert!(
        matches!(server.operation_at(None, server.remote(), &options, Arc::new(AtomicBool::new(false))).0.push_upstream(peer.workdir().unwrap()), Outcome::Refused(m) if m.contains("buffer limit"))
    );
}

#[test]
fn git_sync_tls_names_all_five_targets() {
    // AGENTS §2: Windows takes Schannel, the other four rustls with the OS
    // verifier; any other target fails to compile instead of syncing without TLS.
    let http = include_str!("http.rs");
    assert!(http.contains("#[cfg(target_os = \"windows\")]\n        {\n            builder = builder.tls_backend_native();"));
    let rustls = "#[cfg(any(\n            target_os = \"linux\",\n            target_os = \"macos\",\n            target_os = \"android\",\n            target_os = \"ios\"\n        ))]\n        {\n            let _ = rustls::crypto::ring::default_provider().install_default();\n            builder = builder.tls_backend_rustls();";
    assert!(http.contains(rustls));
    assert!(http.contains(
        "compile_error!(\"git sync declares a TLS transport for each shipped target only\")"
    ));
}
