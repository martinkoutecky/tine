//! L7s5-01/02: synthetic real repositories through owner run and edit verbs.
use super::*;
use crate::owner::git_run::lock_test_hook;
use std::path::Path;

#[cfg(not(target_os = "android"))]
#[test]
fn sb1_run_refuses_the_root_replaced_since_store_open() {
    let graph = Graph::new();
    let original = repo(&graph);
    let original_objects = objects(&original);
    let slot = graph.slot();
    let run = crate::owner::git_run::run_of(&json!({"run": begin(&slot)}))
        .unwrap()
        .unwrap();
    let moved = graph.root.with_extension("moved");
    std::fs::rename(&graph.root, &moved).unwrap();
    std::fs::create_dir(&graph.root).unwrap();
    Repository::init(&graph.root).unwrap();
    let error = crate::owner::git_run::commit_write(
        &slot,
        &run,
        &tine_store::PageId::from("pages/a.md"),
        "Edit",
        None,
        Some(b"- retained input\n"),
    )
    .err()
    .expect("detached run refuses before any repository mutation");
    assert!(
        error.message.contains("GraphRootMoved"),
        "{}",
        error.message
    );
    assert!(!graph.root.join(".git/index").exists());
    assert!(objects(&Repository::open(&graph.root).unwrap()).is_empty());
    assert_eq!(
        objects(&Repository::open(&moved).unwrap()),
        original_objects
    );
}

fn objects(repo: &Repository) -> Vec<git2::Oid> {
    let mut ids = Vec::new();
    repo.odb()
        .unwrap()
        .foreach(|id| {
            ids.push(*id);
            true
        })
        .unwrap();
    ids.sort();
    ids
}

#[test]
fn run_conversion_refuses_before_save_and_objects() {
    for nested in [false, true] {
        for attr in [
            "filter=crypt",
            "filter=lfs",
            "ident",
            "working-tree-encoding=UTF-16",
        ] {
            let graph = Graph::new();
            let root = if nested {
                graph.root.parent().unwrap()
            } else {
                &graph.root
            };
            let repo = Repository::init(root).unwrap();
            commit_all(&repo, "initial");
            let slot = graph.slot();
            let run = begin(&slot);
            let path = if nested {
                "graph/pages/dup.md"
            } else {
                "pages/dup.md"
            };
            // This path is absent from autocommit's representative config samples.
            std::fs::write(root.join(".gitattributes"), format!("{path} {attr}\n")).unwrap();
            let before = disk(&graph, "pages/dup.md");
            let tip = head(&repo).id();
            let index = std::fs::read(repo.path().join("index")).unwrap();
            let ids = objects(&repo);
            let result = verb(
                &slot,
                "block.edit",
                json!({
                    "handle": handle(&slot, "dup", 0), "text": "private edited text", "run": run
                }),
            );
            assert!(
                result.is_err(),
                "{nested} {attr}: run must refuse before saving: {result:?}"
            );
            let error = result.unwrap_err();
            assert!(error.message.contains(path), "{}", error.message);
            assert_eq!(error.code, "refused");
            assert_eq!(disk(&graph, "pages/dup.md"), before);
            assert_eq!(head(&repo).id(), tip);
            assert_eq!(std::fs::read(repo.path().join("index")).unwrap(), index);
            assert_eq!(objects(&repo), ids, "no raw snapshot or agent object");

            // The policy is exact-path, not a blanket refusal of this repository.
            let reply = edit(&slot, &handle(&slot, "b", 0), "supported edit", Some(&run));
            assert_eq!(reply["git"]["committed"], true, "{reply}");
        }
    }
}

#[test]
fn run_commit_boundary_refuses_conversion_before_objects() {
    let graph = Graph::new();
    let repo = repo(&graph);
    let slot = graph.slot();
    let id = begin(&slot);
    // Simulate conversion introduced after pre-save admission.
    std::fs::write(
        graph.root.join(".gitattributes"),
        "pages/a.md filter=crypt\n",
    )
    .unwrap();
    let before = disk(&graph, "pages/a.md");
    let tip = head(&repo).id();
    let index = std::fs::read(repo.path().join("index")).unwrap();
    let ids = objects(&repo);
    let run = crate::owner::git_run::run_of(&json!({"run": id}))
        .unwrap()
        .unwrap();
    let result = crate::owner::git_run::commit_write(
        &slot,
        &run,
        &tine_store::PageId::from("pages/a.md"),
        "Edit",
        Some(before.as_bytes()),
        Some(b"- confidential new content\n"),
    );
    assert!(
        result.is_err(),
        "conversion must refuse before creating any raw object"
    );
    assert_eq!(disk(&graph, "pages/a.md"), before);
    assert_eq!(head(&repo).id(), tip);
    assert_eq!(std::fs::read(repo.path().join("index")).unwrap(), index);
    assert_eq!(objects(&repo), ids);
}

#[test]
fn run_checkout_before_index_lock_is_saved_but_uncommitted() {
    let graph = Graph::new();
    let slot = graph.slot();
    let repo = repo(&graph);
    let branch_a = repo.head().unwrap().name().unwrap().to_string();
    let tip_a = head(&repo).id();
    // B differs on another page. Its target-page entry still matches A, so
    // staged-kept cannot mask accidental staging of the agent blob into B.
    edit(&slot, &handle(&slot, "b", 0), "branch B", None);
    commit_all(&repo, "B content");
    let tip_b = head(&repo).id();
    repo.branch("other", &head(&repo), false).unwrap();
    repo.reference(&branch_a, tip_a, true, "restore A").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    let run = begin(&slot);
    let address = handle(&slot, "a", 0);
    let repo_path = repo.path().to_path_buf();
    let (opened_tx, opened_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    lock_test_hook::install(move || {
        opened_tx.send(()).unwrap();
        resume_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    });
    let checkout = std::thread::spawn(move || {
        opened_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let repo = Repository::open(repo_path).unwrap();
        repo.set_head("refs/heads/other").unwrap();
        repo.checkout_head(None).unwrap();
        let index = std::fs::read(repo.path().join("index")).unwrap();
        resume_tx.send(()).unwrap();
        index
    });
    let reply = edit(&slot, &address, "agent on A", Some(&run));
    let b_index = checkout.join().unwrap();
    assert!(
        disk(&graph, "pages/a.md").contains("agent on A"),
        "the saved edit survives checkout"
    );
    assert_eq!(reply["git"]["committed"], false, "{reply}");
    assert!(
        reply["git"]["message"].as_str().unwrap().contains("branch"),
        "{reply}"
    );
    assert_eq!(
        repo.refname_to_id(&branch_a).unwrap(),
        tip_a,
        "A must not advance"
    );
    assert_eq!(repo.refname_to_id("refs/heads/other").unwrap(), tip_b);
    assert_eq!(std::fs::read(repo.path().join("index")).unwrap(), b_index);
    assert!(!repo.path().join("index.lock").exists());
    let mut index = repo.index().unwrap();
    index.read(true).unwrap();
    assert_eq!(
        index.get_path(Path::new("pages/a.md"), 0).unwrap().id,
        head(&repo)
            .tree()
            .unwrap()
            .get_path(Path::new("pages/a.md"))
            .unwrap()
            .id()
    );
}

#[test]
fn run_false_unspecified_and_eol_attributes_keep_exact_bytes() {
    for attributes in [
        "pages/a.md -filter -ident -working-tree-encoding\n",
        "pages/a.md !filter !ident !working-tree-encoding\n",
        "pages/a.md text eol=crlf\n",
    ] {
        let graph = Graph::new();
        let repo = repo(&graph);
        let slot = graph.slot();
        let run = begin(&slot);
        std::fs::write(graph.root.join(".gitattributes"), attributes).unwrap();
        let reply = edit(&slot, &handle(&slot, "a", 0), "exact bytes", Some(&run));
        assert_eq!(reply["git"]["committed"], true, "{attributes}: {reply}");
        assert_eq!(
            file_at(&repo, &head(&repo), "pages/a.md"),
            disk(&graph, "pages/a.md")
        );
    }
}

#[test]
fn autocommit_checkout_uses_the_same_branch_ownership_rule() {
    let graph = Graph::new();
    let repo = repo(&graph);
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    repo.config()
        .unwrap()
        .set_bool("core.precomposeunicode", true)
        .unwrap();
    let branch_a = repo.head().unwrap().name().unwrap().to_string();
    let tip = head(&repo).id();
    repo.branch("other", &head(&repo), false).unwrap();
    std::fs::write(graph.root.join("pages/a.md"), "- pending changes\n").unwrap();
    let index = std::fs::read(repo.path().join("index")).unwrap();
    let repo_path = repo.path().to_path_buf();
    lock_test_hook::install(move || {
        // Deterministic checkout between open and lock; preserve pending changes.
        let repo = Repository::open(repo_path).unwrap();
        repo.set_head("refs/heads/other").unwrap();
        repo.checkout_head(None).unwrap();
    });
    let result = crate::owner::git_autocommit::commit_now(&graph.root, "autosave");
    let error = result.expect_err("a checkout must not silently retry onto B");
    assert!(error.message().contains("branch"), "{error:?}");
    assert_eq!(repo.refname_to_id(&branch_a).unwrap(), tip);
    assert_eq!(repo.refname_to_id("refs/heads/other").unwrap(), tip);
    assert_eq!(std::fs::read(repo.path().join("index")).unwrap(), index);
    assert_eq!(disk(&graph, "pages/a.md"), "- pending changes\n");
}
