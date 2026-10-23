//! Reviewed transcripts and deterministic generated-page proofs (CLI spec §3).
use super::*;
use crate::owner::terminal::{error_json, error_text, exit_code, render};

#[test]
fn rn_f_cli_edit_preserves_one_running_clock_and_closes_it() {
    for org in [false, true] {
        for seconds in [false, true] {
            for (marker, text) in [("DOING", "DOING work"), ("DOING", "DONE work"), ("NOW", "NOW work"), ("NOW", "DONE work")] {
                let graph = Graph::new();
                let rel = if org { "pages/task.org" } else { "pages/task.md" };
                let start = if seconds { "2026-01-01 Thu 09:00:00" } else { "2026-01-01 Thu 09:00" };
                let source = if org {
                    format!("* {marker}\n:PROPERTIES:\n:id: {UUID}\n:END:\n:LOGBOOK:\nCLOCK: [{start}]\n:END:\n")
                } else {
                    format!("- {marker}\n  id:: {UUID}\n  :LOGBOOK:\n  CLOCK: [{start}]\n  :END:\n")
                };
                std::fs::write(graph.root.join("pages/a.md"), "- other\n").unwrap();
                std::fs::write(graph.root.join(rel), source).unwrap();
                std::fs::write(graph.root.join("logseq/config.edn"), format!(
                    "{{:file/name-format :triple-lowbar :feature/enable-timetracking? true :logbook/settings {{:with-second-support? {seconds}}}}}\n"
                )).unwrap();
                graph.store.refresh(tine_store::Depth::Bytes).unwrap();
                let slot = graph.slot();
                // block.edit replaces the entire editor buffer, including its logbook.
                let edited = format!("{text}\n:LOGBOOK:\nCLOCK: [{start}]\n:END:");
                verb(&slot, "block.edit", json!({"handle": UUID, "text": edited})).unwrap();
                let written = std::fs::read_to_string(graph.root.join(rel)).unwrap();
                assert_eq!(written.matches("CLOCK:").count(), 1, "{org}/{seconds}/{text}: {written}");
                assert_eq!(written.contains("]--["), text == "DONE work", "{org}/{seconds}/{text}: {written}");
            }
        }
    }
}

fn cli_command(verb: &str, args: &Value) -> Vec<String> {
    let get = |key: &str| args[key].as_str().unwrap().to_string();
    let mut command = vec!["tine".to_string()];
    match verb {
        "block.edit" => command.extend(["edit".into(), get("handle"), get("text")]),
        "block.delete" => command.extend(["delete".into(), get("handle")]),
        "block.prop" => {
            command.extend([
                "prop".into(),
                if args["value"].is_null() {
                    "remove"
                } else {
                    "set"
                }
                .into(),
                get("handle"),
                get("key"),
            ]);
            if !args["value"].is_null() {
                command.push(get("value"));
            }
        }
        "block.move" => command.extend([
            "move".into(),
            get("handle"),
            format!("--{}", get("at")),
            get("to"),
        ]),
        "block.add" => {
            command.push("add".into());
            for key in ["page", "under", "before", "after", "author"] {
                if args[key].is_string() {
                    command.extend([format!("--{key}"), get(key)]);
                }
            }
            command.push(get("content"));
        }
        _ => unreachable!(),
    }
    if args["rev"].is_string() {
        command.extend(["--rev".into(), get("rev")]);
    }
    command.extend(["--graph".into(), "$GRAPH".into()]);
    command
}

fn fixture(input: &str, org: bool) -> (Graph, GraphSlot, PathBuf) {
    let graph = Graph::new();
    let path = graph.root.join(if org {
        "pages/proof.org"
    } else {
        "pages/proof.md"
    });
    std::fs::write(&path, input).unwrap();
    std::fs::write(graph.root.join("pages/a.md"), "- unrelated\n").unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = graph.slot();
    (graph, slot, path)
}

#[test]
fn write_transcript_corpus() {
    let mut corpus = Vec::new();
    for org in [false, true] {
        let input = if org {
            format!("* one\n:PROPERTIES:\n:id: {UUID}\n:END:\n** child\n* two\n* same\n* same\n")
        } else {
            format!("- one\n  id:: {UUID}\n\t- child\n- two\n- same\n- same\n")
        };
        let mut cases = vec![
            ("edit", "block.edit", json!({"handle": UUID, "text": "ONE"})),
            (
                "prop-set",
                "block.prop",
                json!({"handle": UUID, "key": "status", "value": "ready"}),
            ),
            (
                "prop-remove",
                "block.prop",
                json!({"handle": UUID, "key": "id"}),
            ),
            (
                "move",
                "block.move",
                json!({"handle": UUID, "to": "$two", "at": "after"}),
            ),
            ("delete", "block.delete", json!({"handle": UUID})),
            (
                "add",
                "block.add",
                json!({"page": "proof", "content": if org {"* new\n** nested"} else {"- new\n\t- nested"}, "author": "proof"}),
            ),
            (
                "stale-candidates",
                "block.edit",
                json!({"handle": "$same", "text": "changed"}),
            ),
            (
                "stale-rev",
                "block.delete",
                json!({"handle": UUID, "rev": "old"}),
            ),
            (
                "not-found",
                "block.edit",
                json!({"handle": "absent/1~aaaaaa", "text": "x"}),
            ),
            (
                "bad-args",
                "block.prop",
                json!({"handle": "proof/1", "key": "status", "value": "x"}),
            ),
            (
                "cross-page",
                "block.move",
                json!({"handle": UUID, "to": "$other", "at": "under"}),
            ),
            (
                "cycle",
                "block.move",
                json!({"handle": UUID, "to": "$child", "at": "under"}),
            ),
            (
                "round-trip",
                "block.edit",
                json!({"handle": UUID, "text": if org {"x\n* accidental"} else {"x\n- accidental"}}),
            ),
            (
                "broken-ref",
                "block.add",
                json!({"page": "proof", "content": "((00000000-0000-4000-8000-000000000001))", "author": "proof"}),
            ),
            (
                "bad-placement",
                "block.add",
                json!({"page": "proof", "under": UUID, "content": "x"}),
            ),
            (
                "bad-property",
                "block.prop",
                json!({"handle": UUID, "key": "bad\nkey", "value": "x"}),
            ),
            (
                "add-under",
                "block.add",
                json!({"under": UUID, "content": if org {"* new"} else {"- new"}, "author": "proof"}),
            ),
            (
                "depth-cap",
                "block.add",
                json!({"under": UUID, "content": (0..128).map(|depth| if org { format!("{} node {depth}\n", "*".repeat(depth + 1)) } else { format!("{}- node {depth}\n", "\t".repeat(depth)) }).collect::<String>(), "author": "proof"}),
            ),
        ];
        if org {
            cases.push((
                "read-only",
                "block.edit",
                json!({"handle": "proof/1~aaaaaa", "text": "x"}),
            ));
        }
        for (name, command, mut args) in cases {
            let input = if name == "read-only" {
                "* a\n*** skipped level\n".to_string()
            } else {
                input.clone()
            };
            let (_graph, slot, path) = fixture(&input, org);
            let page = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
            let other = verb(&slot, "page.show", json!({"name": "b"})).unwrap();
            for value in args.as_object_mut().unwrap().values_mut() {
                let replacement = match value.as_str() {
                    Some("$two") => Some(format!("proof/{}", block_handle(&page, 1))),
                    Some("$same") => Some(format!("proof/{}", block_handle(&page, 2))),
                    Some("$child") => Some(format!(
                        "proof/{}",
                        page["page"]["blocks"][0]["children"][0]["handle"]
                            .as_str()
                            .unwrap()
                    )),
                    Some("$other") => Some(format!("b/{}", block_handle(&other, 0))),
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    *value = json!(replacement);
                }
            }
            let (output, json_output, exit) = match verb(&slot, command, args.clone()) {
                Ok(ok) => {
                    let mut text = String::new();
                    render(&ok, &mut text);
                    (text, ok, 0)
                }
                Err(body) => (
                    error_text(&body),
                    serde_json::from_str::<Value>(&error_json(&body)).unwrap(),
                    exit_code(&body.code),
                ),
            };
            corpus.push(json!({"case": format!("{}-{name}", if org {"org"} else {"md"}), "command": cli_command(command, &args), "request": {"verb": command, "args": args}, "input": input, "text": output, "json": json_output, "exit": exit, "after": std::fs::read_to_string(path).unwrap()}));
        }
        // Volatile I/O outcomes are injected at the typed host-result boundary,
        // then pass through the production mapping and terminal rendering. These
        // pin output, not filesystem fault recovery (the host has its own tests).
        let page = PageId::from(if org {
            "pages/proof.org"
        } else {
            "pages/proof.md"
        });
        for (name, outcome) in [
            ("changed-twice", tine_store::ApplyRefusal::Changed),
            ("unsaved", tine_store::ApplyRefusal::Unsaved(page.clone())),
            ("page-busy", tine_store::ApplyRefusal::Busy),
            ("host", tine_store::ApplyRefusal::Refused),
            ("draft-failed", tine_store::ApplyRefusal::DraftFailed),
            (
                "unwritten",
                tine_store::ApplyRefusal::Unwritten(page.clone()),
            ),
            (
                "superseded",
                tine_store::ApplyRefusal::Superseded(page.clone()),
            ),
            ("uncertain", tine_store::ApplyRefusal::Uncertain),
        ] {
            let body = crate::owner::write::apply_refused(outcome, &page);
            corpus.push(json!({"case": format!("{}-host-{name}", if org {"org"} else {"md"}), "command": cli_command("block.edit", &json!({"handle": UUID, "text": "x"})), "request": {"verb": "block.edit", "args": {"handle": UUID, "text": "x"}}, "injected_host_outcome": name, "input": input, "after": input, "text": error_text(&body), "json": serde_json::from_str::<Value>(&error_json(&body)).unwrap(), "exit": exit_code(&body.code)}));
        }
    }
    let actual = serde_json::to_string_pretty(&corpus).unwrap() + "\n";
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/owner/owner_tests/write-transcripts.json");
    if std::env::var("TINE_REGENERATE_WRITE_TRANSCRIPTS").as_deref() == Ok("1") {
        std::fs::write(&path, &actual).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        actual,
        "review the transcript diff; regeneration requires TINE_REGENERATE_WRITE_TRANSCRIPTS=1"
    );
}

fn shape(blocks: &[tine_core::model::BlockDto]) -> Value {
    json!(blocks
        .iter()
        .map(|b| json!({"raw": b.raw, "children": shape(&b.children)}))
        .collect::<Vec<_>>())
}

// No additional dependency: a fixed LCG seed makes every failure reproducible.
fn generated(verb_name: &str) {
    let mut seed = 0x71ae_2026_u64;
    for org in [false, true] {
        for iteration in 0..40 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let n = 3 + (seed as usize % 7);
            let target = (seed.rotate_left(17) as usize) % n;
            let mut chunks = Vec::new();
            for i in 0..n {
                let depth = 1 + (seed.rotate_left(i as u32) as usize % 3);
                let id = format!("00000000-0000-4000-8000-{:012x}", i + 1);
                let mut chunk = if org {
                    format!("* TODO root {iteration}-{i}\n:PROPERTIES:\n:id: {id}\n:note: keep {i}\n:END:\nbody {i}\n")
                } else {
                    format!(
                        "- TODO root {iteration}-{i}\n  id:: {id}\n  note:: keep {i}\n  body {i}\n"
                    )
                };
                for d in 1..=depth {
                    chunk.push_str(&if org {
                        format!("{} child {i}-{d}\n", "*".repeat(d + 1))
                    } else {
                        format!("{}- child {i}-{d}\n", "\t".repeat(d))
                    });
                }
                chunks.push(chunk);
            }
            let input = chunks.concat();
            let (graph, slot, path) = fixture(&input, org);
            let page = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
            let handle = format!("proof/{}", block_handle(&page, target));
            let mut expected = chunks.clone();
            let args = match verb_name {
                "edit" => {
                    let text = format!("DONE changed {iteration}\nsecond line");
                    // Construct the editor projection independently: hidden id survives,
                    // visible note/body are replaced; descendants are untouched.
                    let id = format!("00000000-0000-4000-8000-{:012x}", target + 1);
                    let child_at = chunks[target]
                        .find(if org { "** child" } else { "\t- child" })
                        .unwrap();
                    expected[target] = if org {
                        format!("* DONE changed {iteration}\n:PROPERTIES:\n:id: {id}\n:END:\nsecond line\n{}", &chunks[target][child_at..])
                    } else {
                        format!(
                            "- DONE changed {iteration}\n  second line\n  id:: {id}\n{}",
                            &chunks[target][child_at..]
                        )
                    };
                    json!({"handle": handle, "text": text})
                }
                "prop" => {
                    let remove = iteration % 2 == 0;
                    expected[target] = chunks[target].replace(
                        &if org {
                            format!(":note: keep {target}\n")
                        } else {
                            format!("  note:: keep {target}\n")
                        },
                        if remove {
                            ""
                        } else if org {
                            ":note: changed\n"
                        } else {
                            "  note:: changed\n"
                        },
                    );
                    json!({"handle": handle, "key": "note", "value": if remove {Value::Null} else {json!("changed")}})
                }
                "move" => {
                    let to = (target + 1) % n;
                    let moved = expected.remove(target);
                    let dest = if to > target { to - 1 } else { to };
                    expected.insert(dest, moved);
                    json!({"handle": handle, "to": format!("proof/{}", block_handle(&page, to)), "at": "before"})
                }
                "delete" => {
                    expected.remove(target);
                    json!({"handle": handle})
                }
                "add" => {
                    expected.push(if org {"* added\n:PROPERTIES:\n:author: proof\n:END:\n** nested\n:PROPERTIES:\n:author: proof\n:END:\n".into()} else {"- added\n  author:: proof\n\t- nested\n\t  author:: proof\n".into()});
                    json!({"page": "proof", "content": if org {"* added\n** nested"} else {"- added\n\t- nested"}, "author": "proof"})
                }
                _ => unreachable!(),
            };
            verb(&slot, &format!("block.{verb_name}"), args)
                .unwrap_or_else(|e| panic!("seed {seed:x} {org} {verb_name}: {e:?}"));
            let bytes = std::fs::read(&path).unwrap();
            let id = PageId::from(if org {
                "pages/proof.org"
            } else {
                "pages/proof.md"
            });
            let actual = graph.store.page_of(&id, &bytes).unwrap().doc;
            let intended = graph
                .store
                .page_of(&id, expected.concat().as_bytes())
                .unwrap()
                .doc;
            assert_eq!(
                shape(&actual.blocks),
                shape(&intended.blocks),
                "{verb_name} org={org} seed={seed:x}"
            );
            let written = String::from_utf8(bytes).unwrap();
            for (i, chunk) in chunks.iter().enumerate() {
                if i != target || verb_name == "add" || verb_name == "move" {
                    assert!(written.contains(chunk), "untouched subtree lost exact file bytes: {verb_name} org={org} seed={seed:x} block={i}");
                }
                if i == target && matches!(verb_name, "edit" | "prop") {
                    let child_at = chunk
                        .find(if org { "** child" } else { "\t- child" })
                        .unwrap();
                    assert!(written.contains(&chunk[child_at..]), "untouched descendants lost exact file bytes: {verb_name} org={org} seed={seed:x}");
                }
            }
        }
    }
}

#[test]
fn generated_edit() {
    generated("edit");
}
#[test]
fn generated_prop() {
    generated("prop");
}
#[test]
fn generated_move() {
    generated("move");
}
#[test]
fn generated_delete() {
    generated("delete");
}
#[test]
fn generated_add() {
    generated("add");
}
