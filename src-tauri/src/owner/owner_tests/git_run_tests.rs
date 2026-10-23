//! CLI git runs (slice 3, D-20261019-CL1…CL4) against a real store, page
//! host and git repository: per-write commits with the user's snapshot
//! first, `run end`'s squash conditions, and `run revert`.

use git2::{Repository, Signature};
use serde_json::{json, Value};

use super::{block_handle, error_code, verb, Graph, UUID};

#[path = "git_run_safety_tests.rs"]
mod safety;

/// The graph as a repository with everything committed once, as the user's.
fn repo(graph: &Graph) -> Repository {
    let repo = Repository::init(&graph.root).unwrap();
    {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Martin").unwrap();
        config.set_str("user.email", "martin@example.org").unwrap();
    }
    commit_all(&repo, "initial");
    repo
}

fn commit_all(repo: &Repository, message: &str) {
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let me = Signature::now("Martin", "martin@example.org").unwrap();
    let parent = repo.head().ok().map(|h| h.peel_to_commit().unwrap());
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(Some("HEAD"), &me, &me, message, &tree, &parents)
        .unwrap();
}

fn head(repo: &Repository) -> git2::Commit<'_> {
    repo.head().unwrap().peel_to_commit().unwrap()
}

fn file_at(repo: &Repository, commit: &git2::Commit<'_>, path: &str) -> String {
    let entry = commit
        .tree()
        .unwrap()
        .get_path(std::path::Path::new(path))
        .unwrap();
    let blob = repo.find_blob(entry.id()).unwrap();
    String::from_utf8(blob.content().to_vec()).unwrap()
}

fn disk(graph: &Graph, path: &str) -> String {
    std::fs::read_to_string(graph.root.join(path)).unwrap()
}

fn begin(slot: &crate::state::GraphSlot) -> String {
    let begun = verb(slot, "run.begin", json!({})).unwrap();
    assert_eq!(begun["kind"], "run");
    begun["run"].as_str().unwrap().to_string()
}

fn handle(slot: &crate::state::GraphSlot, page: &str, index: usize) -> String {
    let read = verb(slot, "page.show", json!({ "name": page })).unwrap();
    format!("{page}/{}", block_handle(&read, index))
}

fn edit(slot: &crate::state::GraphSlot, handle: &str, text: &str, run: Option<&str>) -> Value {
    verb(
        slot,
        "block.edit",
        json!({ "handle": handle, "text": text, "run": run, "author": "claude" }),
    )
    .unwrap()
}

fn reason(detail: &Value) -> &str {
    detail["reason"].as_str().unwrap_or("")
}

#[test]
fn a_run_needs_a_usable_repository_and_refuses_before_writing() {
    let graph = Graph::new();
    let slot = graph.slot();
    let (code, detail) = error_code(verb(&slot, "run.begin", json!({})));
    assert_eq!((code.as_str(), reason(&detail)), ("refused", "git"));
    let before = disk(&graph, "pages/a.md");
    let one = handle(&slot, "a", 0);
    let (code, detail) = error_code(verb(
        &slot,
        "block.edit",
        json!({ "handle": one, "text": "x", "run": "r6713a2bc-0123abcd" }),
    ));
    assert_eq!((code.as_str(), reason(&detail)), ("refused", "git"));
    assert_eq!(disk(&graph, "pages/a.md"), before, "nothing was written");

    let (code, _) = error_code(verb(
        &slot,
        "block.edit",
        json!({ "handle": one, "text": "x", "run": "not-a-run" }),
    ));
    assert_eq!(code, "bad-args");

    // A detached HEAD cannot take commits on a branch.
    let repo = repo(&graph);
    let tip = head(&repo).id();
    repo.set_head_detached(tip).unwrap();
    let (code, detail) = error_code(verb(&slot, "run.begin", json!({})));
    assert_eq!((code.as_str(), reason(&detail)), ("refused", "git"));
}

#[test]
fn each_run_write_commits_its_bytes_after_a_snapshot_of_the_users_edits() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let initial = head(&repo).id();
    let run = begin(&slot);

    // HEAD already has the bytes the write starts from: one agent commit.
    let written = edit(
        &slot,
        &handle(&slot, "a", 1),
        "two, by the agent",
        Some(&run),
    );
    assert_eq!(written["git"]["committed"], true, "{written}");
    assert!(written["git"]["snapshot"].is_null());
    let agent = head(&repo);
    assert_eq!(agent.parent_id(0).unwrap(), initial);
    assert_eq!(agent.author().name().unwrap(), "claude");
    assert_eq!(agent.author().email().unwrap(), "agent@tine.invalid");
    assert!(
        agent
            .message()
            .unwrap()
            .contains(&format!("\n\nTine-Run: {run}\n")),
        "{:?}",
        agent.message()
    );
    assert_eq!(
        file_at(&repo, &agent, "pages/a.md"),
        disk(&graph, "pages/a.md")
    );
    let status = repo
        .status_file(std::path::Path::new("pages/a.md"))
        .unwrap();
    assert!(
        status.is_empty(),
        "the index follows the commit: {status:?}"
    );
    assert!(
        !repo.path().join("index.lock").exists(),
        "the index lock is released"
    );
    let leftovers: Vec<_> = std::fs::read_dir(repo.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("tine-index-"))
        .collect();
    assert!(leftovers.is_empty(), "no scratch index left behind");

    // The user's own edit (not in a run), then the agent's next write: the
    // user's bytes are committed first, under the user's name.
    edit(&slot, &handle(&slot, "a", 0), "one, by Martin", None);
    let users = disk(&graph, "pages/a.md");
    let written = edit(&slot, &handle(&slot, "a", 1), "two, again", Some(&run));
    let snapshot = written["git"]["snapshot"].as_str().expect("a snapshot");
    let agent2 = head(&repo);
    let snap = repo
        .find_commit(git2::Oid::from_str(snapshot).unwrap())
        .unwrap();
    assert_eq!(agent2.parent_id(0).unwrap(), snap.id());
    assert_eq!(snap.parent_id(0).unwrap(), agent.id());
    assert_eq!(snap.author().name().unwrap(), "Martin");
    assert_eq!(file_at(&repo, &snap, "pages/a.md"), users);
    assert!(file_at(&repo, &agent2, "pages/a.md").contains("two, again"));

    let shown = verb(&slot, "run.show", json!({ "run": run })).unwrap();
    let agents: Vec<bool> = shown["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["agent"].as_bool().unwrap())
        .collect();
    assert_eq!(agents, vec![true, false, true]);
}

#[test]
fn run_end_squashes_only_a_contiguous_unpublished_run() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let initial = head(&repo).id();

    // Contiguous at the tip: one commit, same tree, parent before the run.
    let run = begin(&slot);
    edit(&slot, &handle(&slot, "a", 1), "two, 1", Some(&run));
    edit(&slot, &handle(&slot, "a", 1), "two, 2", Some(&run));
    let tip = head(&repo);
    let ended = verb(&slot, "run.end", json!({ "run": run, "message": "Tidy a" })).unwrap();
    let squashed = head(&repo);
    assert_eq!(
        ended["squashed"].as_str(),
        Some(squashed.id().to_string().as_str()),
        "{ended}"
    );
    assert_eq!(squashed.parent_id(0).unwrap(), initial);
    assert_eq!(squashed.tree_id(), tip.tree_id());
    assert!(squashed.message().unwrap().starts_with("Tidy a\n"));
    assert!(squashed
        .message()
        .unwrap()
        .contains(&format!("Tine-Run: {run}")));

    // A user commit in between: kept as is.
    let run = begin(&slot);
    edit(&slot, &handle(&slot, "a", 1), "two, 3", Some(&run));
    std::fs::write(graph.root.join("notes.txt"), "mine\n").unwrap();
    commit_all(&repo, "my notes");
    edit(&slot, &handle(&slot, "a", 1), "two, 4", Some(&run));
    let before = head(&repo).id();
    let ended = verb(&slot, "run.end", json!({ "run": run })).unwrap();
    assert!(ended["squashed"].is_null(), "{ended}");
    assert_eq!(head(&repo).id(), before);

    // Pushed (a remote-tracking ref reaches it): kept as is.
    let run = begin(&slot);
    edit(&slot, &handle(&slot, "a", 1), "two, 5", Some(&run));
    let first = head(&repo).id();
    repo.reference("refs/remotes/origin/main", first, true, "test")
        .unwrap();
    edit(&slot, &handle(&slot, "a", 1), "two, 6", Some(&run));
    let before = head(&repo).id();
    let ended = verb(&slot, "run.end", json!({ "run": run })).unwrap();
    assert!(ended["squashed"].is_null(), "{ended}");
    assert!(ended["kept_because"]
        .as_str()
        .unwrap()
        .contains("remotes/origin/main"));
    assert_eq!(head(&repo).id(), before);
}

#[test]
fn run_revert_undoes_the_agents_edits_and_keeps_what_the_user_changed_since() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let original = disk(&graph, "pages/a.md");
    let run = begin(&slot);
    edit(
        &slot,
        &handle(&slot, "a", 1),
        "two, by the agent",
        Some(&run),
    );
    verb(
        &slot,
        "block.add",
        json!({ "page": "b", "content": "- agent's note", "run": run, "author": "claude" }),
    )
    .unwrap();
    // The user edits another block of a since.
    edit(&slot, &handle(&slot, "a", 0), "one, by Martin", None);

    let reverted = verb(&slot, "run.revert", json!({ "run": run })).unwrap();
    let pages = reverted["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 2, "{reverted}");
    assert!(pages.iter().all(|p| p["reverted"] == true), "{reverted}");
    let a = disk(&graph, "pages/a.md");
    assert!(a.contains("- one, by Martin\n"), "{a}");
    assert!(a.contains("- two\n") && !a.contains("by the agent"), "{a}");
    assert!(a.contains(&format!("id:: {UUID}")), "{a}");
    assert!(!disk(&graph, "pages/b.md").contains("agent's note"));
    assert_ne!(a, original);
    let last = head(&repo);
    assert!(last
        .message()
        .unwrap()
        .contains(&format!("Tine-Run-Revert: {run}")));

    // A block the user rewrote after the agent: the user's text stays, listed.
    // (Texts close enough for Concord to pair the block across the edits;
    // far-apart texts read as a deletion plus an insertion, run_revert docs.)
    edit(
        &slot,
        &handle(&slot, "a", 1),
        "the second block of page a",
        None,
    );
    let run = begin(&slot);
    edit(
        &slot,
        &handle(&slot, "a", 1),
        "the second block of page a!",
        Some(&run),
    );
    edit(
        &slot,
        &handle(&slot, "a", 1),
        "the second block of page a!?",
        None,
    );
    let reverted = verb(&slot, "run.revert", json!({ "run": run })).unwrap();
    let page = &reverted["pages"][0];
    let a = disk(&graph, "pages/a.md");
    assert!(a.contains("- the second block of page a!?\n"), "{a}");
    assert_eq!(a.matches("the second block").count(), 1, "{a}");
    assert_eq!(page["kept"].as_array().map(Vec::len), Some(1), "{reverted}");
    assert_eq!(page["kept"][0]["reason"], "changed-since");
}

#[test]
fn content_the_user_staged_stays_staged() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let original = disk(&graph, "pages/a.md");
    // Stage a version of a, then put the file back as it is at HEAD: the
    // staged version is now only in the index.
    edit(&slot, &handle(&slot, "a", 1), "staged by Martin", None);
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("pages/a.md")).unwrap();
    index.write().unwrap();
    let staged = index
        .get_path(std::path::Path::new("pages/a.md"), 0)
        .unwrap()
        .id;
    assert!(original.ends_with("\n- two\n"), "{original}");
    edit(&slot, &handle(&slot, "a", 1), "two", None);
    assert_eq!(disk(&graph, "pages/a.md"), original, "back to HEAD's bytes");

    let run = begin(&slot);
    let written = edit(
        &slot,
        &handle(&slot, "a", 1),
        "two, by the agent",
        Some(&run),
    );
    assert_eq!(written["git"]["committed"], true, "{written}");
    assert_eq!(written["git"]["index"], "staged-kept", "{written}");
    let mut index = repo.index().unwrap();
    index.read(true).unwrap();
    assert_eq!(
        index
            .get_path(std::path::Path::new("pages/a.md"), 0)
            .unwrap()
            .id,
        staged,
        "the staged version is still staged"
    );
}

#[test]
fn a_held_index_lock_refuses_the_write_before_it_changes_anything() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let run = begin(&slot);
    let lock = repo.path().join("index.lock");
    std::fs::write(&lock, "").unwrap();
    let before = disk(&graph, "pages/a.md");
    let tip = head(&repo).id();
    let (code, detail) = error_code(verb(
        &slot,
        "block.edit",
        json!({ "handle": handle(&slot, "a", 1), "text": "x", "run": run }),
    ));
    std::fs::remove_file(&lock).unwrap();
    assert_eq!((code.as_str(), reason(&detail)), ("refused", "git-busy"));
    assert_eq!(disk(&graph, "pages/a.md"), before);
    assert_eq!(head(&repo).id(), tip);
}

#[test]
fn a_run_id_quoted_in_a_users_message_does_not_make_the_commit_the_runs() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let run = begin(&slot);
    edit(
        &slot,
        &handle(&slot, "a", 1),
        "two, by the agent",
        Some(&run),
    );
    std::fs::write(graph.root.join("pages/user.md"), "- mine\n").unwrap();
    commit_all(
        &repo,
        &format!("Notes on the agent's run\n\nIt was:\nTine-Run: {run}\nand that is all I have to say.\n"),
    );
    let shown = verb(&slot, "run.show", json!({ "run": run })).unwrap();
    assert_eq!(
        shown["commits"].as_array().map(Vec::len),
        Some(1),
        "{shown}"
    );
    let reverted = verb(&slot, "run.revert", json!({ "run": run })).unwrap();
    let pages: Vec<&str> = reverted["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["page"].as_str().unwrap())
        .collect();
    assert_eq!(pages, ["pages/a.md"], "{reverted}");
    assert_eq!(disk(&graph, "pages/user.md"), "- mine\n");
}

#[test]
fn a_revert_is_a_run_that_reverting_undoes() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let run = begin(&slot);
    edit(
        &slot,
        &handle(&slot, "a", 1),
        "two, by the agent",
        Some(&run),
    );
    let by_agent = disk(&graph, "pages/a.md");
    let reverted = verb(&slot, "run.revert", json!({ "run": run })).unwrap();
    let again = reverted["revert_run"]
        .as_str()
        .expect("a revert run")
        .to_string();
    assert_ne!(again, run);
    let message = head(&repo).message().unwrap().to_string();
    assert!(
        message.contains(&format!("Tine-Run: {again}\n")),
        "{message}"
    );
    assert!(
        message.contains(&format!("Tine-Run-Revert: {run}\n")),
        "{message}"
    );
    assert!(!disk(&graph, "pages/a.md").contains("by the agent"));
    verb(&slot, "run.revert", json!({ "run": again })).unwrap();
    assert_eq!(disk(&graph, "pages/a.md"), by_agent);
}

#[test]
fn a_held_index_lock_refuses_a_revert_before_it_changes_anything() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let run = begin(&slot);
    edit(
        &slot,
        &handle(&slot, "a", 1),
        "two, by the agent",
        Some(&run),
    );
    let lock = repo.path().join("index.lock");
    std::fs::write(&lock, "").unwrap();
    let before = disk(&graph, "pages/a.md");
    let tip = head(&repo).id();
    let (code, detail) = error_code(verb(&slot, "run.revert", json!({ "run": run })));
    std::fs::remove_file(&lock).unwrap();
    assert_eq!((code.as_str(), reason(&detail)), ("refused", "git-busy"));
    assert_eq!(disk(&graph, "pages/a.md"), before);
    assert_eq!(head(&repo).id(), tip);
}
