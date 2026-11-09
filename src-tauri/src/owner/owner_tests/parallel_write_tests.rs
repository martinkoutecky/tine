//! GH #842: CLI writes to one page in one owner, in parallel, against a real
//! owner over its socket. A write that names no revision lands; `--rev` and a
//! handle's fingerprint still refuse; an outside writer is never overwritten.

use std::sync::Barrier;

use serde_json::{json, Value};

use super::super::client::Failure;
use super::{block_handle, verb, Graph};

pub(super) const WRITERS: usize = 6;
pub(super) const ROUNDS: usize = 12;

pub(super) fn disk(graph: &Graph, path: &str) -> String {
    std::fs::read_to_string(graph.root.join(path)).unwrap()
}

/// One verb per argument set, released together, each on its own connection
/// as separate `tine` processes are.
pub(super) fn at_once(graph: &Graph, verb: &str, args: &[Value]) -> Vec<Result<Value, Failure>> {
    let start = Barrier::new(args.len());
    std::thread::scope(|scope| {
        let threads: Vec<_> = args
            .iter()
            .map(|args| {
                let start = &start;
                scope.spawn(move || {
                    let mut conn = graph.connect();
                    start.wait();
                    conn.call(verb, args.clone())
                })
            })
            .collect();
        threads.into_iter().map(|t| t.join().unwrap()).collect()
    })
}

pub(super) fn all_landed(results: &[Result<Value, Failure>], what: &str) {
    let failed: Vec<_> = results.iter().filter(|r| r.is_err()).collect();
    assert!(failed.is_empty(), "{what}: {failed:?}");
}

pub(super) fn failure_code(result: &Result<Value, Failure>) -> &str {
    match result {
        Err(Failure::Owner(body)) => &body.code,
        other => panic!("expected an owner error, got {other:?}"),
    }
}

#[test]
fn gh842_parallel_adds_to_one_page_all_land_once_in_some_order() {
    let graph = Graph::new();
    let slot = std::sync::Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot);
    let mut expected_prefix = disk(&graph, "pages/a.md");
    for round in 0..ROUNDS {
        let args: Vec<Value> = (0..WRITERS)
            .map(|i| json!({ "page": "a", "content": format!("- block {round}-{i}") }))
            .collect();
        all_landed(&at_once(&graph, "block.add", &args), "add");
        let text = disk(&graph, "pages/a.md");
        assert!(
            text.starts_with(&expected_prefix),
            "existing blocks were reordered or lost:\n{text}"
        );
        for i in 0..WRITERS {
            let line = format!("- block {round}-{i}\n");
            assert_eq!(text.matches(&line).count(), 1, "{line}in\n{text}");
        }
        expected_prefix = text;
    }
}

#[test]
fn gh842_parallel_adds_to_an_absent_page_all_land() {
    let graph = Graph::new();
    let slot = std::sync::Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot);
    for round in 0..ROUNDS {
        let name = format!("Parallel {round}");
        let args: Vec<Value> = (0..WRITERS)
            .map(|i| json!({ "page": name, "content": format!("- block {i}") }))
            .collect();
        let results = at_once(&graph, "block.add", &args);
        all_landed(&results, "add");
        let id = results[0].as_ref().unwrap()["page"].as_str().unwrap();
        let text = disk(&graph, id);
        for i in 0..WRITERS {
            let line = format!("- block {i}\n");
            assert_eq!(text.matches(&line).count(), 1, "{line}in\n{text}");
        }
    }
}

/// A page of `WRITERS` blocks with their handles, `id::`-free.
fn six_blocks(name: &str, slot: &crate::state::GraphSlot) -> Vec<String> {
    let content: String = (0..WRITERS).map(|i| format!("- item {i}\n")).collect();
    verb(
        slot,
        "block.add",
        json!({ "page": name, "content": content }),
    )
    .unwrap();
    let read = verb(slot, "page.show", json!({ "name": name })).unwrap();
    (0..WRITERS)
        .map(|i| format!("{name}/{}", block_handle(&read, i)))
        .collect()
}

#[test]
fn gh842_block_verbs_under_parallel_writes_land() {
    let graph = Graph::new();
    let slot = std::sync::Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot.clone());
    for round in 0..ROUNDS {
        let name = format!("Verbs {round}");
        let page = format!("pages/Verbs {round}.md");

        let handles = six_blocks(&name, &slot);
        let props: Vec<Value> = handles
            .iter()
            .enumerate()
            .map(|(i, h)| json!({ "handle": h, "key": "k", "value": i.to_string() }))
            .collect();
        all_landed(&at_once(&graph, "block.prop", &props), "prop");
        // A visible property is part of the fingerprint: read again, as a caller
        // must. `ref`'s hidden `id::` and a move leave it alone.
        let read = verb(&slot, "page.show", json!({ "name": name })).unwrap();
        let handles: Vec<String> = (0..WRITERS)
            .map(|i| format!("{name}/{}", block_handle(&read, i)))
            .collect();
        let refs: Vec<Value> = handles.iter().map(|h| json!({ "handle": h })).collect();
        all_landed(&at_once(&graph, "ref", &refs), "ref");
        assert_eq!(disk(&graph, &page).matches("id:: ").count(), WRITERS);
        assert_eq!(disk(&graph, &page).matches("k:: ").count(), WRITERS);

        let moves: Vec<Value> = handles[1..]
            .iter()
            .map(|h| json!({ "handle": h, "to": handles[0], "at": "under" }))
            .collect();
        all_landed(&at_once(&graph, "block.move", &moves), "move");
        assert_eq!(
            disk(&graph, &page).matches("\t- item ").count(),
            WRITERS - 1
        );

        let edits: Vec<Value> = handles
            .iter()
            .enumerate()
            .map(|(i, h)| json!({ "handle": h, "text": format!("edited {i}") }))
            .collect();
        all_landed(&at_once(&graph, "block.edit", &edits), "edit");
        let text = disk(&graph, &page);
        for i in 0..WRITERS {
            assert_eq!(text.matches(&format!("edited {i}\n")).count(), 1, "{text}");
        }
        assert_eq!(text.matches("k:: ").count(), WRITERS, "{text}");

        let other = format!("Deleted {round}");
        let handles = six_blocks(&other, &slot);
        let deletes: Vec<Value> = handles.iter().map(|h| json!({ "handle": h })).collect();
        all_landed(&at_once(&graph, "block.delete", &deletes), "delete");
        let text = disk(&graph, &format!("pages/Deleted {round}.md"));
        assert!(!text.contains("item"), "{text}");
    }
}

#[test]
fn gh842_a_page_delete_and_parallel_adds_all_land() {
    let graph = Graph::new();
    let slot = std::sync::Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot.clone());
    for round in 0..ROUNDS {
        let name = format!("Gone {round}");
        verb(
            &slot,
            "block.add",
            json!({ "page": name, "content": "- first" }),
        )
        .unwrap();
        let start = Barrier::new(WRITERS + 1);
        let (deleted, added) = std::thread::scope(|scope| {
            let adds: Vec<_> = (0..WRITERS)
                .map(|i| {
                    let (graph, start, name) = (&graph, &start, &name);
                    scope.spawn(move || {
                        let mut conn = graph.connect();
                        start.wait();
                        conn.call(
                            "block.add",
                            json!({ "page": name, "content": format!("- block {i}") }),
                        )
                    })
                })
                .collect();
            let mut conn = graph.connect();
            start.wait();
            let deleted = conn.call("page.delete", json!({ "name": name, "yes": true }));
            let added: Vec<_> = adds.into_iter().map(|t| t.join().unwrap()).collect();
            (deleted, added)
        });
        deleted.expect("a page delete with no --rev lands");
        all_landed(&added, "an add recreates the page after the delete");
    }
}

/// `page rename` rewrites other pages' references, so it takes the whole graph's
/// turn: a command writing a referrer neither loses to it nor makes it fail.
#[test]
fn gh842_a_rename_and_parallel_adds_to_its_referrer_all_land() {
    for _ in 0..ROUNDS {
        let graph = Graph::new();
        let slot = std::sync::Arc::new(graph.slot());
        let (_endpoint, _gate) = super::endpoint(&graph, slot);
        let start = Barrier::new(WRITERS + 1);
        let (renamed, added) = std::thread::scope(|scope| {
            let adds: Vec<_> = (0..WRITERS)
                .map(|i| {
                    let (graph, start) = (&graph, &start);
                    scope.spawn(move || {
                        let mut conn = graph.connect();
                        start.wait();
                        conn.call(
                            "block.add",
                            json!({ "page": "b", "content": format!("- block {i}") }),
                        )
                    })
                })
                .collect();
            let mut conn = graph.connect();
            start.wait();
            let renamed = conn.call("page.rename", json!({ "old": "a", "new": "a renamed" }));
            let added: Vec<_> = adds.into_iter().map(|t| t.join().unwrap()).collect();
            (renamed, added)
        });
        let renamed = renamed.expect("the rename lands");
        all_landed(&added, "adds to the referrer");
        let text = disk(&graph, "pages/b.md");
        for i in 0..WRITERS {
            assert!(text.contains(&format!("- block {i}\n")), "{text}");
        }
        let skipped = renamed["report"]["skipped_conflicted_referrers"]
            .as_array()
            .unwrap();
        assert!(
            skipped.is_empty() && text.contains("[[a renamed]]"),
            "{renamed}\n{text}"
        );
    }
}

#[test]
fn gh842_rev_still_refuses_every_writer_but_the_first() {
    let graph = Graph::new();
    let slot = std::sync::Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot.clone());
    for round in 0..ROUNDS {
        let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
        let rev = read["rev"].as_str().unwrap();
        let args: Vec<Value> = (0..WRITERS)
            .map(|i| json!({ "page": "a", "rev": rev, "content": format!("- rev {round}-{i}") }))
            .collect();
        let results = at_once(&graph, "block.add", &args);
        let landed = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(landed, 1, "{results:?}");
        let text = disk(&graph, "pages/a.md");
        for (i, result) in results.iter().enumerate() {
            let line = format!("- rev {round}-{i}\n");
            if result.is_ok() {
                assert!(text.contains(&line));
            } else {
                assert_eq!(failure_code(result), "stale", "{result:?}");
                assert!(!text.contains(&line));
            }
        }
    }
}

#[test]
fn gh842_a_handle_still_refuses_once_a_parallel_writer_changed_its_block() {
    let graph = Graph::new();
    let slot = std::sync::Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot.clone());
    let handles = six_blocks("Same block", &slot);
    let args: Vec<Value> = (0..WRITERS)
        .map(|i| json!({ "handle": handles[0], "text": format!("winner {i}") }))
        .collect();
    let results = at_once(&graph, "block.edit", &args);
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    let text = disk(&graph, "pages/Same block.md");
    for (i, result) in results.iter().enumerate() {
        if result.is_err() {
            assert_eq!(failure_code(result), "stale", "{result:?}");
            assert!(!text.contains(&format!("winner {i}")));
        }
    }
}

/// An outside writer (an editor, Syncthing) moves the file between a write's
/// read and its operation, from inside the window `compute` runs in.
fn write_with_outside_changes(graph: &Graph, changes: usize) -> Result<Value, String> {
    let slot = graph.slot();
    let page = tine_store::PageId::from("pages/a.md");
    let mut left = changes;
    let mut count = 0;
    let result = super::super::write::write_page(
        &slot,
        &page,
        tine_store::EditKind::SaveBlock,
        None,
        |doc, _| {
            if left > 0 {
                left -= 1;
                count += 1;
                let path = graph.root.join("pages/a.md");
                let text = std::fs::read_to_string(&path).unwrap();
                std::fs::write(&path, format!("{text}- outside {count}\n")).unwrap();
            }
            let mut next = doc.clone();
            next.blocks[0].raw = format!("{} cli", next.blocks[0].raw);
            Ok(super::super::write::Planned {
                doc: next,
                changed: vec![vec![0]],
            })
        },
    );
    result.map_err(|error| error.code)
}

#[test]
fn gh842_an_outside_writer_is_reread_once_and_then_stale_and_never_overwritten() {
    let graph = Graph::new();
    write_with_outside_changes(&graph, 1).expect("one outside change is read again");
    let text = disk(&graph, "pages/a.md");
    assert!(text.contains("- outside 1\n"), "{text}");
    assert!(text.contains(" cli"), "{text}");

    let graph = Graph::new();
    let code = write_with_outside_changes(&graph, 2).expect_err("a sustained writer");
    assert_eq!(code, "stale");
    let text = disk(&graph, "pages/a.md");
    assert!(text.contains("- outside 1\n"), "{text}");
    assert!(text.contains("- outside 2\n"), "{text}");
    assert!(
        !text.contains(" cli"),
        "the refused write left nothing: {text}"
    );
}

/// A write behind one that never finishes: a refusal the caller can act on,
/// nothing written, whichever verb it is.
#[test]
fn gh842_a_write_past_its_wait_in_line_is_page_busy_and_writes_nothing() {
    let graph = Graph::new();
    let mut slot = graph.slot();
    slot.turn_wait = std::time::Duration::from_millis(100);
    let slot = std::sync::Arc::new(slot);
    let (_endpoint, _gate) = super::endpoint(&graph, slot.clone());
    let before = disk(&graph, "pages/a.md");
    let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
    let second = format!("a/{}", block_handle(&read, 1));
    let wedged = super::super::write::page_turn(&slot, &tine_store::PageId::from("pages/a.md"))
        .unwrap_or_else(|_| panic!("the line is empty"));
    let mut conn = graph.connect();
    for (name, args) in [
        ("block.add", json!({ "page": "a", "content": "- late" })),
        ("block.edit", json!({ "handle": second, "text": "late" })),
        ("page.delete", json!({ "name": "a", "yes": true })),
        ("page.rename", json!({ "old": "a", "new": "late" })),
    ] {
        let result = conn.call(name, args);
        let Err(Failure::Owner(body)) = &result else {
            panic!("{name}: {result:?}");
        };
        assert_eq!(body.code, "refused", "{name}");
        let detail = body.detail.as_ref().unwrap();
        assert_eq!(detail["reason"], "page-busy", "{name}");
        assert_eq!(detail["admitted"], false, "{name}");
        assert_eq!(super::super::terminal::exit_code(&body.code), 6);
        assert_eq!(disk(&graph, "pages/a.md"), before, "{name}");
    }
    assert!(!graph.root.join("pages/late.md").exists());
    drop(wedged);
    conn.call("block.add", json!({ "page": "a", "content": "- after" }))
        .expect("the line moves once the writer ahead finishes");
}

/// A page an earlier command in the line created is the page a rename or delete
/// behind it finds (review F3).
#[test]
fn gh842_rename_and_delete_resolve_their_name_in_their_turn() {
    let graph = Graph::new();
    let slot = std::sync::Arc::new(graph.slot());
    let (_endpoint, _gate) = super::endpoint(&graph, slot.clone());
    for (round, name) in ["block-then-rename", "block-then-delete"]
        .iter()
        .enumerate()
    {
        let wedged =
            super::super::write::graph_turn(&slot).unwrap_or_else(|_| panic!("the line is empty"));
        let (added, moved) = std::thread::scope(|scope| {
            let add = scope.spawn(|| {
                graph
                    .connect()
                    .call("block.add", json!({ "page": name, "content": "- first" }))
            });
            while super::super::write::tests_line_len(&slot) < 2 {
                std::thread::yield_now();
            }
            let moved = scope.spawn(|| {
                if round == 0 {
                    graph.connect().call(
                        "page.rename",
                        json!({ "old": name, "new": format!("{name} renamed") }),
                    )
                } else {
                    graph
                        .connect()
                        .call("page.delete", json!({ "name": name, "yes": true }))
                }
            });
            while super::super::write::tests_line_len(&slot) < 3 {
                std::thread::yield_now();
            }
            drop(wedged);
            (add.join().unwrap(), moved.join().unwrap())
        });
        added.expect("the add lands first");
        let reply = moved.expect("the page exists by its turn");
        if round == 0 {
            assert_eq!(reply["kind"], "page-renamed", "{reply}");
            assert_eq!(reply["report"]["touched"][0]["moved"], true, "{reply}");
            assert!(graph.root.join(format!("pages/{name} renamed.md")).exists());
        } else {
            assert_eq!(reply["kind"], "page-deleted", "{reply}");
        }
        assert!(!graph.root.join(format!("pages/{name}.md")).exists());
    }
}
