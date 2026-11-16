//! Sync cycles (ADR 0078 slice 2) against a real store, page host and a bare
//! repository over `file://`: the cycle itself, the worker's "Sync now", and
//! the policy and its refusals as Settings sees them.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use git2::{Oid, Repository, Signature};

use super::Graph;
use crate::owner::git_autocommit::{
    self, commit_now, git_status, settings_from_json, settings_into_json, GitSettings, Outcome,
    Policy, MESSAGE,
};
use crate::owner::git_sync::{NoCredentials, Operation, Outcome as Net, TransportOptions};
use crate::owner::git_sync_cycle::{cycle, Class};

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
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let parent = repo.head().ok().map(|h| h.peel_to_commit().unwrap());
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(Some("HEAD"), &me(), &me(), message, &tree, &parents)
        .unwrap()
}

fn configure(repo: &Repository) {
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Martin").unwrap();
    config.set_str("user.email", "martin@example.org").unwrap();
    config.set_bool("core.autocrlf", false).unwrap();
    config.set_bool("core.precomposeunicode", true).unwrap();
}

fn head(repo: &Repository) -> Oid {
    repo.head().unwrap().peel_to_commit().unwrap().id()
}

/// The graph as a clone-like repository on `main`, pushed once to a bare
/// remote it tracks over `file://`.
struct Synced {
    graph: Graph,
    repo: Repository,
    bare: Repository,
    url: String,
}

impl Synced {
    fn new() -> Self {
        let graph = Graph::new();
        let mut options = git2::RepositoryInitOptions::new();
        options.initial_head("main");
        let repo = Repository::init_opts(&graph.root, &options).unwrap();
        configure(&repo);
        commit_all(&repo, "initial");
        let bare_path = graph.app_data.parent().unwrap().join("remote.git");
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
        assert_eq!(operation().push_upstream(&graph.root), Net::UpToDate);
        Self {
            graph,
            repo,
            bare,
            url,
        }
    }

    /// Another device: clone, change `path`, push.
    fn peer_pushes(&self, path: &str, text: &str) -> Oid {
        let dir = self
            .graph
            .app_data
            .parent()
            .unwrap()
            .join(format!("peer-{}", uuid::Uuid::new_v4()));
        let peer = Repository::clone(&self.url, &dir).unwrap();
        configure(&peer);
        std::fs::write(dir.join(path), text).unwrap();
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

    fn disk(&self, path: &str) -> String {
        std::fs::read_to_string(self.graph.root.join(path)).unwrap()
    }
}

fn credentials() -> Arc<dyn crate::owner::git_sync::Credentials> {
    Arc::new(NoCredentials)
}

#[test]
fn a_cycle_pushes_pulls_and_stops_at_divergence_without_touching_the_graph() {
    let s = Synced::new();
    let slot = s.graph.slot();
    // Nothing changed on either side: one fetch, no push.
    let quiet = cycle(&slot, &credentials());
    assert_eq!((quiet.class, quiet.code), (Class::Ok, "up-to-date"));

    std::fs::write(s.graph.root.join("pages/b.md"), "- mine\n").unwrap();
    assert!(matches!(
        commit_now(&s.graph.root, MESSAGE),
        Ok(Outcome::Committed(_))
    ));
    let pushed = cycle(&slot, &credentials());
    assert_eq!(pushed.code, "pushed");
    assert_eq!(s.remote_tip(), head(&s.repo));

    let theirs = s.peer_pushes("pages/a.md", "- from the laptop\n");
    let pulled = cycle(&slot, &credentials());
    assert_eq!(pulled.code, "pulled", "{pulled:?}");
    assert_eq!(head(&s.repo), theirs);
    assert_eq!(s.disk("pages/a.md"), "- from the laptop\n");
    let again = cycle(&slot, &credentials());
    assert_eq!(again.code, "up-to-date", "a pull leaves nothing to push");

    // Both sides move: phase 1 stops, the graph and the remote stay as they are.
    let remote = s.peer_pushes("pages/a.md", "- laptop again\n");
    std::fs::write(s.graph.root.join("pages/b.md"), "- mine again\n").unwrap();
    let local = match commit_now(&s.graph.root, MESSAGE) {
        Ok(Outcome::Committed(oid)) => oid,
        other => panic!("{other:?}"),
    };
    let diverged = cycle(&slot, &credentials());
    assert_eq!(
        (diverged.class, diverged.code),
        (Class::NeedsUser, "diverged")
    );
    assert!(diverged.backs_off());
    assert_eq!(head(&s.repo), local);
    assert_eq!(s.remote_tip(), remote);
    assert_eq!(s.disk("pages/a.md"), "- from the laptop\n");
    assert_eq!(s.disk("pages/b.md"), "- mine again\n");
}

#[test]
fn a_cycle_and_settings_name_a_missing_upstream() {
    let s = Synced::new();
    let slot = s.graph.slot();
    s.repo
        .config()
        .unwrap()
        .remove("branch.main.merge")
        .unwrap();
    let missing = cycle(&slot, &credentials());
    assert_eq!((missing.class, missing.code), (Class::NeedsUser, "refused"));
    assert!(missing.message.contains("upstream"), "{missing:?}");
    let status = git_status(&s.graph.root, GitSettings::default());
    assert!(status.remote.is_none());
    assert!(status.sync_unusable.unwrap().1.contains("upstream"));
}

#[test]
fn settings_name_the_remote_and_refuse_sync_for_a_graph_inside_a_bigger_repository() {
    let s = Synced::new();
    let status = git_status(&s.graph.root, GitSettings::default());
    let remote = status.remote.expect("the tracked remote");
    assert_eq!(
        (remote.name.as_str(), remote.url.as_str()),
        ("origin", s.url.as_str())
    );
    assert_eq!(remote.https_origin, None, "a file remote needs no sign-in");
    assert_eq!(status.sync_unusable, None);

    // A graph in a subfolder of a work tree commits but cannot sync.
    let outer = s.graph.app_data.parent().unwrap().join("outer");
    let nested = outer.join("notes");
    std::fs::create_dir_all(nested.join("pages")).unwrap();
    std::fs::write(nested.join("pages/a.md"), "- a\n").unwrap();
    let repo = Repository::init(&outer).unwrap();
    configure(&repo);
    commit_all(&repo, "initial");
    let nested_status = git_status(&nested, GitSettings::default());
    assert_eq!(nested_status.unusable, None, "autocommit still works");
    let why = nested_status.sync_unusable.expect("sync is refused").1;
    assert!(why.contains("work tree root"), "{why}");
}

#[test]
fn sync_policies_round_trip_through_the_device_settings_and_imply_commits() {
    let root = Path::new("/graph");
    let mut json = serde_json::json!({});
    for policy in [
        Policy::Off,
        Policy::Commit,
        Policy::SyncOnDemand,
        Policy::SyncAutomatically,
    ] {
        let settings = GitSettings {
            policy,
            interval: Duration::from_secs(15 * 60),
        };
        settings_into_json(&mut json, root, settings);
        assert_eq!(settings_from_json(&json, root), settings);
        assert_eq!(policy.commits(), policy != Policy::Off);
    }
    assert_eq!(
        json[git_autocommit::SETTINGS_KEY]["/graph"]["policy"],
        "sync-automatically"
    );
    assert!(Policy::SyncOnDemand.syncs() && !Policy::Commit.syncs());
}

#[test]
fn sync_now_commits_pending_edits_then_pushes_on_the_worker_thread() {
    let s = Synced::new();
    let slot = Arc::new(s.graph.slot());
    git_autocommit::start_worker(
        &slot,
        GitSettings {
            policy: Policy::SyncOnDemand,
            interval: Duration::from_secs(10 * 60),
        },
        credentials(),
    );
    std::fs::write(s.graph.root.join("pages/b.md"), "- typed before Sync now\n").unwrap();
    assert!(git_autocommit::request_sync(&s.graph.root));
    let deadline = Instant::now() + Duration::from_secs(60);
    let sync = loop {
        let sync = git_autocommit::last_attempt(&s.graph.root).sync;
        if !sync.running && sync.at.is_some() {
            break sync;
        }
        assert!(
            Instant::now() < deadline,
            "the worker never finished the cycle"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(sync.code.as_deref(), Some("pushed"), "{sync:?}");
    assert!(sync.ok_at.is_some() && !sync.persistent);
    let tip = s.bare.find_commit(s.remote_tip()).unwrap();
    assert_eq!(tip.message().ok(), Some(MESSAGE));
    assert_eq!(tip.id(), head(&s.repo));
    let tree = tip.tree().unwrap();
    let blob = s
        .bare
        .find_blob(tree.get_path(Path::new("pages/b.md")).unwrap().id())
        .unwrap();
    assert_eq!(blob.content(), b"- typed before Sync now\n");
    slot.background_cancelled
        .store(true, std::sync::atomic::Ordering::Release);
}
