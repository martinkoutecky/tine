//! Discussion #960: physical subtree layout after a shallower CLI move.
use super::*;

// Depth counts indentation steps (roots are zero). Before/after also cover
// moving to a root; `under` necessarily lands at depth one or deeper.
fn block(org: bool, unit: &str, depth: usize, raw: &str) -> String {
    if org {
        format!("{} {raw}\n", "*".repeat(depth + 1))
    } else {
        let prefix = unit.repeat(depth);
        format!(
            "{prefix}- {}\n",
            raw.replace('\n', &format!("\n{prefix}  "))
        )
    }
}

pub(super) fn shallower_move_transcripts() -> Vec<Value> {
    let mut transcripts = Vec::new();
    let mut failures = Vec::new();
    for org in [false, true] {
        for unit in ["\t", "  "] {
            for newline in ["\n", "\r\n"] {
                let b = |depth, raw: &str| block(org, unit, depth, raw);
                let child = if org {
                    format!("child\n{unit}:PROPERTIES:\n{unit}:status: ready\n{unit}:END:\n{unit}line one\n{unit}line two")
                } else {
                    "child\nstatus:: ready\nline one\nline two\n:LOGBOOK:\nCLOCK: [2026-11-15 Sun 08:00:00]\n:END:".into()
                };
                let subtree = |depth| {
                    format!(
                        "{}{}{}{}",
                        b(depth, "moving"),
                        b(depth + 1, &child),
                        b(depth + 2, "grandchild"),
                        b(depth + 1, "sibling")
                    )
                };
                let source = format!(
                    "{}{}{}{}{}",
                    b(0, "source"),
                    b(1, "parent"),
                    b(2, "branch"),
                    subtree(3),
                    b(2, "untouched")
                );
                let destination = format!(
                    "{}{}{}{}{}",
                    b(0, "target"),
                    b(1, "nested target"),
                    b(2, "deep target"),
                    b(3, "existing child"),
                    b(0, "tail")
                );
                let input = format!("{source}{destination}").replace('\n', newline);
                let remaining = format!(
                    "{}{}{}{}",
                    b(0, "source"),
                    b(1, "parent"),
                    b(2, "branch"),
                    b(2, "untouched")
                );
                for at in ["under", "after", "before"] {
                    for depth in 0..=2 {
                        if at == "under" && depth == 0 {
                            continue;
                        }
                        let target_depth = if at == "under" { depth - 1 } else { depth };
                        let (graph, slot, path) = fixture(&input, org);
                        let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
                        let moving =
                            &read["page"]["blocks"][0]["children"][0]["children"][0]["children"][0];
                        assert_eq!(moving["raw"], "moving");
                        assert_eq!(moving["children"][0]["children"][0]["raw"], "grandchild");
                        let mut target = &read["page"]["blocks"][1];
                        for _ in 0..target_depth {
                            target = &target["children"][0];
                        }
                        let args = json!({"handle": format!("proof/{}", moving["handle"].as_str().unwrap()), "to": format!("proof/{}", target["handle"].as_str().unwrap()), "at": at});
                        let mut expected = remaining.clone();
                        let targets = ["target", "nested target", "deep target", "existing child"];
                        for (i, raw) in targets.iter().enumerate() {
                            if at == "before" && i == target_depth {
                                expected.push_str(&subtree(depth));
                            }
                            expected.push_str(&b(i, raw));
                            if at == "after" && i == 3 {
                                expected.push_str(&subtree(depth));
                            }
                        }
                        if at == "under" {
                            expected.push_str(&subtree(depth));
                        }
                        expected.push_str(&b(0, "tail"));
                        let expected = expected.replace('\n', newline);
                        let reply = verb(&slot, "block.move", args.clone()).unwrap();
                        let after = std::fs::read_to_string(&path).unwrap();
                        let name = format!(
                            "{}-{}-{}-move-3-to-{depth}-{at}",
                            if org { "org" } else { "md" },
                            if unit == "\t" { "tabs" } else { "spaces" },
                            if newline == "\n" { "lf" } else { "crlf" }
                        );
                        if after != expected {
                            failures.push(format!("{name}: expected {expected:?}, got {after:?}"));
                        }
                        let mut text = String::new();
                        render(&reply, &mut text);
                        transcripts.push(json!({"case": name, "command": cli_command("block.move", &args), "request": {"verb": "block.move", "args": args}, "input": input, "text": text, "json": reply, "exit": 0, "after": after}));
                        drop(graph);
                    }
                }
            }
        }
    }
    assert_eq!(transcripts.len(), 64);
    for newline in ["\n", "\r\n"] {
        let input = "- root\n\t- parent\n\t\t- inner\n  \t\t- child\n  \t\t\t- grandchild\n  \t\t- second child\n- tail\n".replace('\n', newline);
        let (_graph, slot, path) = fixture(&input, false);
        let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
        let args = json!({"handle": format!("proof/{}", block_handle(&read, 0)), "text": "root!"});
        let reply = verb(&slot, "block.edit", args.clone()).unwrap();
        let after = std::fs::read_to_string(path).unwrap();
        assert_eq!(after, input.replacen("root", "root!", 1));
        let mut text = String::new();
        render(&reply, &mut text);
        transcripts.push(json!({"case": format!("md-mixed-prefix-{}-edit-neighbor", if newline == "\n" { "lf" } else { "crlf" }), "command": cli_command("block.edit", &args), "request": {"verb": "block.edit", "args": args}, "input": input, "text": text, "json": reply, "exit": 0, "after": after}));
    }
    assert_eq!(transcripts.len(), 66);
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    transcripts
}

#[test]
fn cli_move_to_shallower_depth_keeps_descendant_file_indentation() {
    shallower_move_transcripts();
}
