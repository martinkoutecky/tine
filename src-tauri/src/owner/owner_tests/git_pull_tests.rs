//! Applying a pull (ADR 0078 §4 rev 2) against a real store, page host and
//! git repository, named after the storage model's scenarios
//! (`docs/storage-model/quint/storage-s37-gitpull.qnt`, qGit*). A crash is a
//! pull stopped at the injected step with its `index.lock` left behind and
//! the P4 set forgotten; a process restart forgets the P4 set.

use git2::{Index, IndexEntry, Oid, Repository, Signature};
use std::path::Path;

use super::Graph;
use crate::owner::git_autocommit::{commit_now, Outcome};
use crate::owner::git_pull::{pull, test_hook, Held, PullOutcome, Step};
use tine_store::{ApplyRefusal, PageId};

const A_AT_H: &str = "- one\n  id:: 6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f\n\t- under one\n- two\n";

#[test]
fn r2_late_publication_after_retry_snapshot_keeps_window_undo() {
    test_hook::clear();
    let graph = Graph::new();
    std::fs::write(graph.root.join("pages/window.md"), "- old\n").unwrap();
    let page = PageId::from("pages/window.md");
    let old = graph.store.page(&page).unwrap().doc;
    let repo = repo(&graph);
    let slot = std::sync::Arc::new(graph.slot());
    let h = head(&repo);
    let up = upstream(&repo, &[Edit::Write("pages/window.md", b"- theirs\n")]);
    let delayed = std::rc::Rc::new(std::cell::RefCell::new(None));
    let captured = delayed.clone();
    test_hook::on_host(move |op| {
        *captured.borrow_mut() = Some(op.clone());
        Some((false, Err(ApplyRefusal::Uncertain)))
    });
    assert!(matches!(pull(&slot), PullOutcome::Incomplete(_)));
    test_hook::clear();
    let mut pending = delayed.borrow_mut().take();
    let window = slot.clone();
    let root = graph.root.clone();
    // The retry has already snapshotted receipts and planned. Publish the
    // older operation, then save a real window undo before retry admission.
    test_hook::on_host(move |_| {
        crate::page_commands::with_host(&window, |host| {
            host.apply(pending.take().unwrap()).unwrap();
            let session = serde_json::to_value(host.window_reloaded()).unwrap();
            let generation = session["session"].as_u64().unwrap();
            let id = session["nextId"].as_u64().unwrap();
            let opened =
                serde_json::to_value(host.open(generation, id, &page, "window").unwrap()).unwrap();
            let version = host.read(&page).unwrap().revision.unwrap().1;
            host.submit(
                generation,
                id + 1,
                opened["key"].as_str().unwrap(),
                &old,
                version,
                None,
                &[tine_store::EditKind::SaveBlock],
            )
            .unwrap();
        })
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::fs::read(root.join("pages/window.md")).unwrap() != b"- old\n" {
            assert!(
                std::time::Instant::now() < deadline,
                "window undo did not save"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(test_hook::applied(&root), ["pages/window.md"]);
        None
    });
    let outcome = pull(&slot);
    assert_eq!(
        disk(&graph, "pages/window.md").unwrap(),
        b"- old\n",
        "a receipt arriving after the retry snapshot must preserve the window undo: {outcome:?}"
    );
    assert!(
        matches!(outcome, PullOutcome::Held(Held::Changed(_))),
        "{outcome:?}"
    );
    assert_eq!(head(&repo), h);
    test_hook::clear();
    assert_eq!(landed(&pull(&slot)), 0);
    assert_eq!(head(&repo), up);
    assert_eq!(disk(&graph, "pages/window.md").unwrap(), b"- old\n");
}

#[test]
fn r2_unicode_case_only_rename_lands_on_sensitive_volume() {
    for folding in [false, true] {
        for edited in [false, true] {
            test_hook::clear();
            let graph = Graph::new();
            std::fs::write(graph.root.join("pages/é.md"), "- unique unicode\n").unwrap();
            graph.store.whole_graph().unwrap();
            let repo = repo(&graph);
            let slot = graph.slot();
            let bytes = if edited {
                b"- unique unicode\n- extra\n".as_slice()
            } else {
                b"- unique unicode\n".as_slice()
            };
            upstream(
                &repo,
                &[Edit::Remove("pages/é.md"), Edit::Write("pages/É.md", bytes)],
            );
            if folding {
                test_hook::simulate_folding();
            }
            test_hook::on_host(|op| {
                assert_eq!(
                    op.moves,
                    [(PageId::from("pages/É.md"), PageId::from("pages/é.md"))]
                );
                assert_eq!(op.pages.len(), 2, "never a source-only deletion");
                None
            });
            let outcome = pull(&slot);
            landed(&outcome);
            test_hook::clear();
            assert_eq!(disk(&graph, "pages/É.md").unwrap(), bytes);
            let listed: Vec<_> = std::fs::read_dir(graph.root.join("pages"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert!(listed.contains(&std::ffi::OsString::from("É.md")));
            assert!(!listed.contains(&std::ffi::OsString::from("é.md")));
        }
    }
}

fn me() -> Signature<'static> {
    Signature::now("Martin", "martin@example.org").unwrap()
}

/// The graph as a repository with everything committed once, its branch
/// tracking `origin` (what a clone or `git push -u` leaves).
fn repo(graph: &Graph) -> Repository {
    let repo = Repository::init(&graph.root).unwrap();
    {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Martin").unwrap();
        config.set_str("user.email", "martin@example.org").unwrap();
    }
    commit_all(&repo, "initial");
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    repo.remote("origin", "/nonexistent/remote.git").unwrap();
    let mut config = repo.config().unwrap();
    config
        .set_str(&format!("branch.{branch}.remote"), "origin")
        .unwrap();
    config
        .set_str(
            &format!("branch.{branch}.merge"),
            &format!("refs/heads/{branch}"),
        )
        .unwrap();
    drop(config);
    repo
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

fn head(repo: &Repository) -> Oid {
    repo.head().unwrap().peel_to_commit().unwrap().id()
}

enum Edit<'a> {
    Write(&'a str, &'a [u8]),
    Mode(&'a str, &'a [u8], u32),
    Remove(&'a str),
}

fn entry(path: &str, mode: u32) -> IndexEntry {
    IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: Oid::ZERO_SHA1,
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    }
}

/// A remote commit on top of `HEAD`, left where a fetch leaves it.
fn upstream(repo: &Repository, edits: &[Edit<'_>]) -> Oid {
    let base = repo.head().unwrap().peel_to_commit().unwrap();
    upstream_on(repo, &base, edits)
}

fn upstream_on(repo: &Repository, base: &git2::Commit<'_>, edits: &[Edit<'_>]) -> Oid {
    let mut index = Index::new().unwrap();
    index.read_tree(&base.tree().unwrap()).unwrap();
    for edit in edits {
        let mut add = |path: &str, bytes: &[u8], mode: u32| {
            let mut e = entry(path, mode);
            e.id = repo.blob(bytes).unwrap();
            e.file_size = bytes.len() as u32;
            index.add(&e)
        };
        match edit {
            Edit::Write(path, bytes) => add(path, bytes, 0o100644),
            Edit::Mode(path, bytes, mode) => add(path, bytes, *mode),
            Edit::Remove(path) => index.remove_path(Path::new(path)),
        }
        .unwrap();
    }
    let tree = repo.find_tree(index.write_tree_to(repo).unwrap()).unwrap();
    let commit = repo
        .commit(None, &me(), &me(), "remote", &tree, &[base])
        .unwrap();
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    repo.reference(
        &format!("refs/remotes/origin/{branch}"),
        commit,
        true,
        "fetch",
    )
    .unwrap();
    commit
}

fn tree_of(repo: &Repository, commit: Oid) -> Oid {
    repo.find_commit(commit).unwrap().tree_id()
}

/// What the index would commit now.
fn index_tree(repo: &Repository) -> Oid {
    let mut index = repo.index().unwrap();
    index.read(true).unwrap();
    index.write_tree().unwrap()
}

fn disk(graph: &Graph, path: &str) -> Option<Vec<u8>> {
    std::fs::read(graph.root.join(path)).ok()
}

fn at_u(repo: &Repository, up: Oid, path: &str) -> Option<Vec<u8>> {
    let tree = repo.find_commit(up).unwrap().tree().unwrap();
    let entry = tree.get_path(Path::new(path)).ok()?;
    Some(repo.find_blob(entry.id()).unwrap().content().to_vec())
}

/// Paths git sees as changed, Tine's own folders left out.
fn dirty(repo: &Repository) -> Vec<String> {
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    repo.statuses(Some(&mut options))
        .unwrap()
        .iter()
        .filter_map(|e| e.path().ok().map(str::to_string))
        .filter(|p| !p.starts_with("logseq/.tine-"))
        .collect()
}

fn landed(outcome: &PullOutcome) -> usize {
    match outcome {
        PullOutcome::Landed { written, .. } => *written,
        other => panic!("expected the pull to land, got {other:?}"),
    }
}

fn remove_lock(repo: &Repository) {
    std::fs::remove_file(repo.path().join("index.lock")).unwrap();
}

fn trashed(graph: &Graph, kind: &str, name: &str) -> bool {
    std::fs::read_dir(graph.root.join("logseq/.tine-trash").join(kind))
        .map(|dir| {
            dir.flatten()
                .any(|e| e.file_name().to_string_lossy().ends_with(name))
        })
        .unwrap_or(false)
}

#[test]
fn q_git_pull_lands_a_fast_forward() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let h = head(&repo);
    let config = b"{:file/name-format :triple-lowbar\n :preferred-format \"markdown\"}\n";
    let up = upstream(
        &repo,
        &[
            Edit::Write("pages/a.md", b"- one, from the laptop\n- two\n"),
            Edit::Write("pages/c.md", b"- new on the laptop\n"),
            Edit::Remove("pages/dup.md"),
            Edit::Write("assets/pic.png", &[0x89, b'P', b'N', b'G', 0, 1, 2, 255]),
            Edit::Write("logseq/config.edn", config),
        ],
    );
    let outcome = pull(&slot);
    assert_eq!(landed(&outcome), 5);
    assert_eq!(
        outcome,
        PullOutcome::Landed {
            from: h,
            to: up,
            written: 5
        }
    );
    assert_eq!(head(&repo), up);
    assert_eq!(index_tree(&repo), tree_of(&repo, up));
    for path in [
        "pages/a.md",
        "pages/c.md",
        "pages/dup.md",
        "assets/pic.png",
        "logseq/config.edn",
    ] {
        assert_eq!(disk(&graph, path), at_u(&repo, up, path), "{path}");
    }
    assert!(
        trashed(&graph, "pages", "dup.md"),
        "a pulled deletion keeps the bytes"
    );
    assert_eq!(dirty(&repo), Vec::<String>::new());
    assert_eq!(pull(&slot), PullOutcome::UpToDate);
}

#[test]
fn q_git_up_to_date_no_upstream_and_diverged() {
    test_hook::clear();
    let graph = Graph::new();
    let slot = graph.slot();
    {
        let bare = Repository::init(&graph.root).unwrap();
        commit_all(&bare, "initial");
    }
    assert_eq!(pull(&slot), PullOutcome::NoUpstream);
    let _ = std::fs::remove_dir_all(graph.root.join(".git"));
    let repo = repo(&graph);
    let h = head(&repo);
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    repo.reference(&format!("refs/remotes/origin/{branch}"), h, true, "fetch")
        .unwrap();
    assert_eq!(pull(&slot), PullOutcome::UpToDate);
    // Both sides moved: the remote's commit and a local one on `H`.
    upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    std::fs::write(graph.root.join("pages/b.md"), "- mine\n").unwrap();
    commit_all(&repo, "local");
    let local = head(&repo);
    assert!(matches!(pull(&slot), PullOutcome::Diverged(_)));
    assert_eq!(head(&repo), local);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), A_AT_H.as_bytes());
}

#[test]
fn q_git_unsaved_input_holds_then_lands() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let h = head(&repo);
    let up = upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    test_hook::on_host(|_| {
        Some((
            false,
            Err(ApplyRefusal::Unsaved(PageId::from("pages/a.md"))),
        ))
    });
    assert_eq!(
        pull(&slot),
        PullOutcome::Held(Held::Unsaved("pages/a.md".into()))
    );
    assert_eq!(head(&repo), h);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), A_AT_H.as_bytes());
    test_hook::clear();
    landed(&pull(&slot));
    assert_eq!(head(&repo), up);
}

#[test]
fn q_git_page_changed_after_the_pre_pull_autocommit_holds_then_diverges() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let h = head(&repo);
    upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    let root = graph.root.clone();
    // Another editor saves the page after the autocommit, before the write.
    test_hook::on_host(move |_| {
        std::fs::write(root.join("pages/a.md"), "- mine, just now\n").unwrap();
        Some((true, Ok(())))
    });
    let outcome = pull(&slot);
    assert!(
        matches!(outcome, PullOutcome::Held(Held::Changed(_))),
        "{outcome:?}"
    );
    test_hook::clear();
    assert_eq!(head(&repo), h);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), b"- mine, just now\n");
    // The next cycle's autocommit records it; the remote has other commits.
    assert!(matches!(pull(&slot), PullOutcome::Diverged(_)));
    assert_ne!(head(&repo), h);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), b"- mine, just now\n");
}

/// The model's crash after the index install and before the branch moves
/// (P1, P1b): the index names `U`'s blobs under `HEAD` = `H`.
fn crash_after_index(
    graph: &Graph,
    repo: &Repository,
    slot: &crate::state::GraphSlot,
) -> (Oid, Oid) {
    let h = head(repo);
    let up = upstream(
        repo,
        &[
            Edit::Write("pages/a.md", b"- theirs\n"),
            Edit::Write("pages/c.md", b"- new\n"),
            Edit::Remove("pages/dup.md"),
        ],
    );
    test_hook::crash_at(Step::AfterIndex);
    assert!(matches!(pull(slot), PullOutcome::Incomplete(_)));
    assert!(repo.path().join("index.lock").exists());
    assert_eq!(head(repo), h);
    assert_eq!(index_tree(repo), tree_of(repo, up));
    test_hook::restart(&graph.root);
    (h, up)
}

#[test]
fn q_git_crash_between_index_and_ref_resumes() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let (_, up) = crash_after_index(&graph, &repo, &slot);
    // The next cycle: the stale lock holds it until the user removes it.
    assert_eq!(
        pull(&slot),
        PullOutcome::Held(Held::Busy("index.lock".into()))
    );
    remove_lock(&repo);
    // P3: the cadence (and every other) autocommit skips the half-applied
    // tree instead of committing it on `H`.
    assert_eq!(
        commit_now(&graph.root, "Tine: autosave").unwrap(),
        Outcome::PullPending
    );
    assert_eq!(landed(&pull(&slot)), 0);
    assert_eq!(head(&repo), up);
    assert_eq!(dirty(&repo), Vec::<String>::new());
}

#[test]
fn q_git_plain_commit_after_the_crash_records_no_reversion() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let (h, up) = crash_after_index(&graph, &repo, &slot);
    remove_lock(&repo);
    // The user's own `git commit` in between commits what the index names.
    let tree = repo.find_tree(index_tree(&repo)).unwrap();
    let parent = repo.find_commit(h).unwrap();
    let plain = repo
        .commit(Some("HEAD"), &me(), &me(), "by hand", &tree, &[&parent])
        .unwrap();
    assert_eq!(
        tree_of(&repo, plain),
        tree_of(&repo, up),
        "no reversion recorded"
    );
    assert!(matches!(pull(&slot), PullOutcome::Diverged(_)));
}

#[test]
fn q_git_every_crash_point_keeps_head_and_index_consistent() {
    for step in [Step::AfterPages, Step::AfterBarrier, Step::AfterIndex] {
        test_hook::clear();
        let graph = Graph::new();
        let repo = repo(&graph);
        let slot = graph.slot();
        let h = head(&repo);
        let up = upstream(
            &repo,
            &[
                Edit::Write("pages/a.md", b"- theirs\n"),
                Edit::Write("assets/n.txt", b"note\n"),
            ],
        );
        test_hook::crash_at(step);
        assert!(
            matches!(pull(&slot), PullOutcome::Incomplete(_)),
            "{step:?}"
        );
        // The branch never moves ahead of the index (the model's reversion).
        assert_eq!(head(&repo), h, "{step:?}");
        let _ = std::fs::remove_file(repo.path().join("index.lock"));
        test_hook::restart(&graph.root);
        assert_eq!(
            commit_now(&graph.root, "Tine: autosave").unwrap(),
            Outcome::PullPending,
            "{step:?}"
        );
        landed(&pull(&slot));
        assert_eq!(head(&repo), up, "{step:?}");
        assert_eq!(disk(&graph, "assets/n.txt").unwrap(), b"note\n");
        assert_eq!(dirty(&repo), Vec::<String>::new(), "{step:?}");
    }
}

#[test]
fn q_git_resume_after_uncertain() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let up = upstream(
        &repo,
        &[
            Edit::Write("pages/a.md", b"- theirs\n"),
            Edit::Write("assets/n.txt", b"note\n"),
        ],
    );
    test_hook::on_host(|_| Some((true, Err(ApplyRefusal::Uncertain))));
    assert!(matches!(pull(&slot), PullOutcome::Incomplete(_)));
    test_hook::clear();
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), b"- theirs\n");
    test_hook::restart(&graph.root);
    // Only the asset is left; the page is already at `U`.
    assert_eq!(landed(&pull(&slot)), 1);
    assert_eq!(head(&repo), up);
}

#[test]
fn q_git_edit_back_to_h_within_the_process_is_kept() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let up = upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    test_hook::crash_at(Step::BarrierFails);
    assert!(matches!(pull(&slot), PullOutcome::Incomplete(_)));
    assert_eq!(
        test_hook::applied(&graph.root),
        vec!["pages/a.md".to_string()]
    );
    // The user restores the old text before the next cycle (P4).
    std::fs::write(graph.root.join("pages/a.md"), A_AT_H).unwrap();
    landed(&pull(&slot));
    assert_eq!(head(&repo), up);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), A_AT_H.as_bytes());
    // The restored text is the user's change on top of `U`.
    assert!(matches!(
        commit_now(&graph.root, "Tine: autosave").unwrap(),
        Outcome::Committed(_)
    ));
}

#[test]
fn q_git_foreign_staged_blob_refuses() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let h = head(&repo);
    upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    let mut index = repo.index().unwrap();
    index
        .add_frombuffer(&entry("pages/a.md", 0o100644), b"- staged by hand\n")
        .unwrap();
    index.write().unwrap();
    let outcome = pull(&slot);
    assert!(
        matches!(&outcome, PullOutcome::Refused { path, .. } if path == "pages/a.md"),
        "{outcome:?}"
    );
    assert_eq!(head(&repo), h);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), A_AT_H.as_bytes());
}

#[test]
fn q_git_page_rename_lands_and_resumes() {
    for (edited, half_done) in [(false, false), (true, false), (false, true)] {
        test_hook::clear();
        let graph = Graph::new();
        let repo = repo(&graph);
        let slot = graph.slot();
        let old = disk(&graph, "pages/b.md").unwrap();
        let mut new = old.clone();
        if edited {
            new.extend_from_slice(b"- one more line\n");
        }
        let up = upstream(
            &repo,
            &[Edit::Remove("pages/b.md"), Edit::Write("pages/b2.md", &new)],
        );
        if half_done {
            // The receiver written, the source not yet deleted.
            std::fs::write(graph.root.join("pages/b2.md"), &new).unwrap();
        }
        landed(&pull(&slot));
        assert_eq!(head(&repo), up);
        assert_eq!(disk(&graph, "pages/b.md"), None, "{edited} {half_done}");
        assert_eq!(
            disk(&graph, "pages/b2.md").unwrap(),
            new,
            "{edited} {half_done}"
        );
        assert!(trashed(&graph, "pages", "b.md"), "{edited} {half_done}");
        assert_eq!(dirty(&repo), Vec::<String>::new());
    }
}

#[test]
fn q_git_barrier_syncs_paths_already_at_u() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    test_hook::on_host(|_| Some((true, Err(ApplyRefusal::Uncertain))));
    assert!(matches!(pull(&slot), PullOutcome::Incomplete(_)));
    test_hook::clear();
    test_hook::restart(&graph.root);
    let _ = tine_store::directory_durability::take_synced_directories();
    assert_eq!(landed(&pull(&slot)), 0);
    let synced = tine_store::directory_durability::take_synced_directories();
    let pages = graph.root.join("pages");
    assert!(
        synced.iter().any(|d| d == &pages),
        "the skipped page's folder is made durable before the ref moves: {synced:?}"
    );
}

#[test]
fn q_git_refuses_by_name_before_writing() {
    let cases: Vec<(&str, Vec<Edit<'_>>, &str)> = vec![
        (
            "symlink",
            vec![Edit::Mode("pages/l.md", b"a.md", 0o120000)],
            "pages/l.md",
        ),
        (
            "mode",
            vec![Edit::Mode("pages/a.md", A_AT_H.as_bytes(), 0o100755)],
            "pages/a.md",
        ),
        (
            "tine-own",
            vec![Edit::Write("logseq/.tine-trash/pages/x.md", b"- x\n")],
            "logseq/.tine-trash/pages/x.md",
        ),
        (
            "file-to-folder",
            vec![
                Edit::Remove("pages/dup.md"),
                Edit::Write("pages/dup.md/x.md", b"- x\n"),
            ],
            "pages/dup.md/x.md",
        ),
    ];
    for (name, edits, path) in cases {
        test_hook::clear();
        let graph = Graph::new();
        let repo = repo(&graph);
        let slot = graph.slot();
        let h = head(&repo);
        // A second, ordinary change that must not be written either.
        let mut edits = edits;
        edits.push(Edit::Write("pages/b.md", b"- theirs\n"));
        upstream(&repo, &edits);
        let before = disk(&graph, "pages/b.md");
        let outcome = pull(&slot);
        assert!(
            matches!(&outcome, PullOutcome::Refused { path: p, .. } if p == path),
            "{name}: {outcome:?}"
        );
        assert_eq!(head(&repo), h, "{name}");
        assert_eq!(disk(&graph, "pages/b.md"), before, "{name}");
    }
}

#[test]
fn q_git_ignored_local_file_in_the_way_refuses() {
    test_hook::clear();
    let graph = Graph::new();
    std::fs::write(graph.root.join(".gitignore"), "pages/ign.md\n").unwrap();
    let repo = repo(&graph);
    let slot = graph.slot();
    std::fs::write(graph.root.join("pages/ign.md"), "- mine, ignored\n").unwrap();
    upstream(&repo, &[Edit::Write("pages/ign.md", b"- theirs\n")]);
    let outcome = pull(&slot);
    assert!(
        matches!(&outcome, PullOutcome::Refused { path, reason } if path == "pages/ign.md" && reason.contains("ignored")),
        "{outcome:?}"
    );
    assert_eq!(disk(&graph, "pages/ign.md").unwrap(), b"- mine, ignored\n");
}

#[test]
fn q_git_publication_is_remembered_before_host_returns() {
    test_hook::clear();
    let graph = Graph::new();
    std::fs::write(graph.root.join("pages/window.md"), "- old\n").unwrap();
    let page = PageId::from("pages/window.md");
    let old = graph.store.page(&page).unwrap().doc;
    let repo = repo(&graph);
    let slot = std::sync::Arc::new(graph.slot());
    let up = upstream(&repo, &[Edit::Write("pages/window.md", b"- theirs\n")]);
    let root = graph.root.clone();
    let window = slot.clone();
    test_hook::on_after_host(move || {
        // This is inside host_apply, before its caller can inspect disk.
        // Drive the same open/submit protocol as a window, saving back to H.
        crate::page_commands::with_host(&window, |host| {
            let session = serde_json::to_value(host.window_reloaded()).unwrap();
            let generation = session["session"].as_u64().unwrap();
            let id = session["nextId"].as_u64().unwrap();
            let opened =
                serde_json::to_value(host.open(generation, id, &page, "window").unwrap()).unwrap();
            let version = host.read(&page).unwrap().revision.unwrap().1;
            host.submit(
                generation,
                id + 1,
                opened["key"].as_str().unwrap(),
                &old,
                version,
                None,
                &[tine_store::EditKind::SaveBlock],
            )
            .unwrap();
        })
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::fs::read(root.join("pages/window.md")).unwrap() != b"- old\n" {
            assert!(
                std::time::Instant::now() < deadline,
                "window undo did not save"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    test_hook::crash_at(Step::BarrierFails);
    assert!(matches!(pull(&slot), PullOutcome::Incomplete(_)));
    assert_eq!(test_hook::applied(&graph.root), ["pages/window.md"]);
    landed(&pull(&slot));
    assert_eq!(head(&repo), up);
    assert_eq!(disk(&graph, "pages/window.md").unwrap(), b"- old\n");
}

#[test]
fn q_git_publication_after_uncertain_updates_the_retained_sink() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let up = upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    let delayed = std::rc::Rc::new(std::cell::RefCell::new(None));
    let captured = delayed.clone();
    test_hook::on_host(move |op| {
        *captured.borrow_mut() = Some(op.clone());
        Some((false, Err(ApplyRefusal::Uncertain)))
    });
    assert!(matches!(pull(&slot), PullOutcome::Incomplete(_)));
    assert!(test_hook::applied(&graph.root).is_empty());
    test_hook::clear();
    // Owner has returned; the original HostOp still owns its sink. The
    // store's late-slot test separately drives admitted work losing a caller.
    let op = delayed.borrow_mut().take().unwrap();
    crate::page_commands::with_host(&slot, |host| host.apply(op))
        .unwrap()
        .unwrap();
    assert_eq!(test_hook::applied(&graph.root), ["pages/a.md"]);
    std::fs::write(graph.root.join("pages/a.md"), A_AT_H).unwrap();
    landed(&pull(&slot));
    assert_eq!(head(&repo), up);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), A_AT_H.as_bytes());
}

#[test]
fn q_git_pending_target_survives_refetch() {
    for undo in [false, true] {
        test_hook::clear();
        let graph = Graph::new();
        let repo = repo(&graph);
        let slot = graph.slot();
        let h = head(&repo);
        let u1 = upstream(&repo, &[Edit::Write("pages/a.md", b"- first\n")]);
        test_hook::crash_at(Step::BarrierFails);
        assert!(matches!(pull(&slot), PullOutcome::Incomplete(_)));
        if undo {
            std::fs::write(graph.root.join("pages/a.md"), A_AT_H).unwrap();
        }
        let parent = repo.find_commit(u1).unwrap();
        // An empty-tree advance exercises undo; otherwise U2 changes the
        // same page again while the disk still holds U1.
        let edits = if undo {
            vec![]
        } else {
            vec![Edit::Write("pages/a.md", b"- second\n")]
        };
        let u2 = upstream_on(&repo, &parent, &edits);
        let auto = commit_now(&graph.root, "Tine: autosave").unwrap();
        assert!(
            matches!(auto, Outcome::PullPending | Outcome::Clean),
            "{auto:?}"
        );
        assert_eq!(head(&repo), h);
        landed(&pull(&slot));
        assert_eq!(head(&repo), u1, "finish the pending target first");
        assert_eq!(
            disk(&graph, "pages/a.md").unwrap(),
            if undo {
                A_AT_H.as_bytes()
            } else {
                b"- first\n"
            }
        );
        if !undo {
            landed(&pull(&slot));
            assert_eq!(head(&repo), u2);
            assert_eq!(disk(&graph, "pages/a.md").unwrap(), b"- second\n");
        }
    }
}

#[test]
fn q_git_force_push_preserves_the_pending_target() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let h = head(&repo);
    upstream(&repo, &[Edit::Write("pages/a.md", b"- first\n")]);
    test_hook::crash_at(Step::BarrierFails);
    assert!(matches!(pull(&slot), PullOutcome::Incomplete(_)));
    upstream(&repo, &[Edit::Write("pages/a.md", b"- force pushed\n")]);
    assert_eq!(
        commit_now(&graph.root, "Tine: autosave").unwrap(),
        Outcome::PullPending
    );
    let outcome = pull(&slot);
    assert!(
        matches!(outcome, PullOutcome::Diverged(ref why) if why.contains("force push")),
        "{outcome:?}"
    );
    assert_eq!(head(&repo), h);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), b"- first\n");
    assert_eq!(test_hook::applied(&graph.root), ["pages/a.md"]);
}

#[test]
fn q_git_case_only_rename_keeps_both_sides() {
    for folding in [false, true] {
        for edited in [false, true] {
            test_hook::clear();
            let graph = Graph::new();
            let repo = repo(&graph);
            let slot = graph.slot();
            let mut bytes = disk(&graph, "pages/a.md").unwrap();
            if edited {
                bytes.extend_from_slice(b"- extra\n");
            }
            let up = upstream(
                &repo,
                &[
                    Edit::Remove("pages/a.md"),
                    Edit::Write("pages/A.md", &bytes),
                ],
            );
            if folding {
                test_hook::simulate_folding();
            }
            test_hook::on_host(|op| {
                assert_eq!(
                    op.moves,
                    [(PageId::from("pages/A.md"), PageId::from("pages/a.md"))]
                );
                assert_eq!(op.pages.len(), 2, "never a source-only deletion");
                None
            });
            landed(&pull(&slot));
            test_hook::clear();
            assert_eq!(head(&repo), up);
            assert_eq!(disk(&graph, "pages/A.md").unwrap(), bytes);
            let names: Vec<_> = std::fs::read_dir(graph.root.join("pages"))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert!(names.contains(&"A.md".into()));
            assert!(!names.contains(&"a.md".into()));
        }
    }
}

#[test]
fn q_git_all_planning_refusals_precede_page_writes() {
    for bad in ["logseq/config.edn", "pages/z.md"] {
        test_hook::clear();
        let graph = Graph::new();
        let repo = repo(&graph);
        let slot = graph.slot();
        upstream(
            &repo,
            &[
                Edit::Write("pages/a.md", b"- theirs\n"),
                Edit::Write(
                    bad,
                    if bad == "logseq/config.edn" {
                        b"{:pages-directory \"../outside\"}\n"
                    } else {
                        &[0xff]
                    },
                ),
            ],
        );
        let outcome = pull(&slot);
        assert!(
            matches!(&outcome, PullOutcome::Refused { path, .. } if path == bad),
            "{outcome:?}"
        );
        assert_eq!(disk(&graph, "pages/a.md").unwrap(), A_AT_H.as_bytes());
        assert!(test_hook::applied(&graph.root).is_empty());
    }
}

#[test]
fn q_git_later_rename_planning_refusal_precedes_the_first_move() {
    test_hook::clear();
    let graph = Graph::new();
    std::fs::write(graph.root.join("pages/z.md"), "- unique z\n").unwrap();
    std::fs::write(graph.root.join("pages/z2.org"), "* twin\n").unwrap();
    graph.store.whole_graph().unwrap();
    let repo = repo(&graph);
    let slot = graph.slot();
    let b = disk(&graph, "pages/b.md").unwrap();
    upstream(
        &repo,
        &[
            Edit::Remove("pages/b.md"),
            Edit::Write("pages/b2.md", &b),
            Edit::Remove("pages/z.md"),
            Edit::Write("pages/z2.md", b"- unique z\n"),
        ],
    );
    let outcome = pull(&slot);
    assert!(
        matches!(&outcome, PullOutcome::Refused { path, .. }
        if path == "pages/z2.md"),
        "{outcome:?}"
    );
    assert_eq!(disk(&graph, "pages/b.md").unwrap(), b);
    assert_eq!(disk(&graph, "pages/b2.md"), None);
}

#[test]
fn q_git_late_staged_refusal_names_partial_progress() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let h = head(&repo);
    upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    let root = graph.root.clone();
    test_hook::on_before_lock(move || {
        let repo = Repository::open(root).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_frombuffer(&entry("pages/a.md", 0o100644), b"- foreign\n")
            .unwrap();
        index.write().unwrap();
    });
    for _ in 0..2 {
        let outcome = pull(&slot);
        assert!(
            matches!(&outcome, PullOutcome::Stopped { path, reason }
            if path == "pages/a.md" && reason.contains("files were already updated")),
            "{outcome:?}"
        );
        assert_eq!(head(&repo), h);
        assert_eq!(disk(&graph, "pages/a.md").unwrap(), b"- theirs\n");
    }
}

#[test]
fn q_git_checkout_race_is_diverged() {
    test_hook::clear();
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let h = head(&repo);
    upstream(&repo, &[Edit::Write("pages/a.md", b"- theirs\n")]);
    repo.branch("other", &repo.find_commit(h).unwrap(), false)
        .unwrap();
    let root = graph.root.clone();
    test_hook::on_before_lock(move || {
        Repository::open(root)
            .unwrap()
            .set_head("refs/heads/other")
            .unwrap();
    });
    let outcome = pull(&slot);
    assert!(matches!(outcome, PullOutcome::Diverged(_)), "{outcome:?}");
    assert_eq!(head(&repo), h);
    assert_eq!(disk(&graph, "pages/a.md").unwrap(), b"- theirs\n");
    assert!(matches!(pull(&slot), PullOutcome::Diverged(_)));
}

#[test]
fn q_git_pull_inside_a_bigger_work_tree_refuses() {
    test_hook::clear();
    let graph = Graph::new();
    let outer = graph.root.parent().unwrap();
    let repo = Repository::init(outer).unwrap();
    {
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Martin").unwrap();
        config.set_str("user.email", "martin@example.org").unwrap();
    }
    commit_all(&repo, "initial");
    let outcome = pull(&graph.slot());
    assert_eq!(outcome.code(), "refused", "{outcome:?}");
}

/// I-25 unit cost per pull (run with `--ignored --nocapture`): files
/// written, bytes, folders synced and wall time for 10 and 1,000 changed
/// pages.
#[test]
#[ignore]
fn q_git_pull_unit_cost() {
    for n in [10usize, 1000] {
        test_hook::clear();
        let graph = Graph::new();
        for i in 0..n {
            std::fs::write(
                graph.root.join(format!("pages/p{i}.md")),
                format!("- page {i}\n  - a child\n"),
            )
            .unwrap();
        }
        graph.store.whole_graph().unwrap();
        let repo = repo(&graph);
        let slot = graph.slot();
        let texts: Vec<(String, Vec<u8>)> = (0..n)
            .map(|i| {
                (
                    format!("pages/p{i}.md"),
                    format!("- page {i}, edited\n  - a child\n").into_bytes(),
                )
            })
            .collect();
        let edits: Vec<Edit<'_>> = texts.iter().map(|(p, b)| Edit::Write(p, b)).collect();
        upstream(&repo, &edits);
        let bytes: usize = texts.iter().map(|(_, b)| b.len()).sum();
        let _ = tine_store::directory_durability::take_synced_directories();
        let start = std::time::Instant::now();
        let written = landed(&pull(&slot));
        let elapsed = start.elapsed();
        let synced = tine_store::directory_durability::take_synced_directories();
        eprintln!(
            "I-25 pull n={n}: files written {written}, bytes {bytes}, directory syncs {}, {:?}",
            synced.len(),
            elapsed
        );
        assert_eq!(written, n);
    }
}
