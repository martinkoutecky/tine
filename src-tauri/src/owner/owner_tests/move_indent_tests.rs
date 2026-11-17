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

/// Review F1 of #960, through the CLI verbs with positional handles and
/// `--rev`: two parents share their text. Moving the first, or adding a
/// subtree with that text before it, leaves the other parent's child bytes
/// alone, and the moved child keeps its offset from its parent.
#[test]
fn cli_move_and_add_beside_a_repeated_parent_keep_each_parents_children() {
    for newline in ["\n", "\r\n", "\r"] {
        let input = "- dup\n\t\t- childA\n- dup\n\t\t\t- childB\n- target\n".replace('\n', newline);
        // A position without its fingerprint, which `--rev` pins.
        let handle = |read: &Value, i: usize| {
            let handle = read["page"]["blocks"][i]["handle"].as_str().unwrap();
            format!("proof/{}", handle.split('~').next().unwrap())
        };
        for at in ["under", "after", "before"] {
            let (graph, slot, path) = fixture(&input, false);
            let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
            let args = json!({"handle": handle(&read, 0), "to": handle(&read, 2), "at": at, "rev": read["rev"]});
            verb(&slot, "block.move", args).unwrap();
            let expected = match at {
                "under" => "- dup\n\t\t\t- childB\n- target\n\t- dup\n\t\t\t- childA\n",
                "after" => "- dup\n\t\t\t- childB\n- target\n- dup\n\t\t- childA\n",
                _ => "- dup\n\t\t\t- childB\n- dup\n\t\t- childA\n- target\n",
            };
            let after = std::fs::read_to_string(&path).unwrap();
            assert_eq!(after, expected.replace('\n', newline), "{at} {newline:?}");
            drop(graph);
        }
        let (_graph, slot, path) = fixture(&input, false);
        let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
        let args = json!({"before": handle(&read, 0), "content": "- dup\n\t- new-child", "rev": read["rev"], "no_author": true});
        verb(&slot, "block.add", args).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        let id = after
            .split(['\n', '\r'])
            .find_map(|line| line.strip_prefix("  id:: "))
            .unwrap_or_else(|| panic!("no id:: in {after:?}"));
        let expected = format!(
            "- dup\n\t- new-child\n- dup\n  id:: {id}\n\t\t- childA\n- dup\n\t\t\t- childB\n- target\n"
        );
        assert_eq!(after, expected.replace('\n', newline), "add {newline:?}");
    }
}

/// Review 293fa of #960, through the CLI verbs with positional handles and
/// `rev`: the parents share their text and so do (some of) their children.
/// Moving the first parent, twice in a row, or adding a copy of its subtree
/// under it, leaves every block it does not move byte-identical.
#[test]
fn cli_move_and_add_beside_repeated_children_keep_untouched_bytes() {
    let handle = |read: &Value, path: &[usize]| {
        let mut block = &read["page"]["blocks"][path[0]];
        for &i in &path[1..] {
            block = &block["children"][i];
        }
        let handle = block["handle"].as_str().unwrap();
        format!("proof/{}", handle.split('~').next().unwrap())
    };
    let families = [
        ("\t\t- x\n", "\t\t\t- x\n"),
        ("\t\t- x\n\t\t- a\n", "\t\t\t- x\n\t\t\t- b\n"),
        ("\t\t- x\n\t\t\t- g1\n", "\t\t\t- x\n\t\t\t\t\t- g2\n"),
    ];
    let (mut cases, mut failures) = (0, Vec::new());
    let mut check = |name: String, after: String, expected: String| {
        cases += 1;
        if after != expected {
            failures.push(format!("{name}: expected {expected:?}, got {after:?}"));
        }
    };
    for newline in ["\n", "\r\n", "\r"] {
        for (first, second) in families {
            let p1 = format!("- dup\n{first}");
            let p2 = format!("- dup\n{second}");
            let input = format!("{p1}{p2}- target\n").replace('\n', newline);
            let moved = format!("\t- dup\n{}", first.replace("\t\t", "\t\t\t"));
            for at in ["under", "after", "before"] {
                let (graph, slot, path) = fixture(&input, false);
                let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
                let args = json!({"handle": handle(&read, &[0]), "to": handle(&read, &[2]), "at": at, "rev": read["rev"]});
                verb(&slot, "block.move", args).unwrap();
                let expected = match at {
                    "under" => format!("{p2}- target\n{moved}"),
                    "after" => format!("{p2}- target\n{p1}"),
                    _ if first == "\t\t- x\n" => format!("{p1}{p2}- target\n"),
                    _ => format!("{p2}{p1}- target\n"),
                };
                let after = std::fs::read_to_string(&path).unwrap();
                check(
                    format!("{at} {input:?}"),
                    after,
                    expected.replace('\n', newline),
                );
                if at == "under" {
                    // The second move starts from the first one's bytes.
                    let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
                    let args = json!({"handle": handle(&read, &[1, 0]), "to": handle(&read, &[0]), "at": "before", "rev": read["rev"]});
                    verb(&slot, "block.move", args).unwrap();
                    let after = std::fs::read_to_string(&path).unwrap();
                    check(format!("back {input:?}"), after, input.clone());
                }
                drop(graph);
            }
            // The reviewer's case: a copy of the first subtree added under it.
            let (_graph, slot, path) = fixture(&input, false);
            let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
            let content = format!("- dup\n{}", first.trim_end());
            let args = json!({"under": handle(&read, &[0]), "content": content, "rev": read["rev"], "no_author": true});
            verb(&slot, "block.add", args).unwrap();
            let after = std::fs::read_to_string(&path).unwrap();
            let id = after
                .split(['\n', '\r'])
                .find_map(|line| line.strip_prefix("  id:: "))
                .map(|id| format!("  id:: {id}\n"))
                .unwrap_or_default();
            // The copy joins the last child's column; its own children sit one
            // page step (a tab) below it.
            let copy = format!("\t\t- dup\n{}", first.replace("\t\t", "\t\t\t"));
            let expected = format!("- dup\n{id}{first}{copy}{p2}- target\n");
            check(
                format!("add {input:?}"),
                after,
                expected.replace('\n', newline),
            );
        }
    }
    assert_eq!(cases, 45);
    assert!(
        failures.is_empty(),
        "{} of {cases} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Review of the 0.9.1 port (2026-11-17, F1): after two same-shaped parents
/// swap places, moving one of them leaves the other's bytes untouched.
#[test]
fn cli_move_after_a_swap_of_repeated_parents_keeps_the_other_parent() {
    let handle = |read: &Value, i: usize| {
        let handle = read["page"]["blocks"][i]["handle"].as_str().unwrap();
        format!("proof/{}", handle.split('~').next().unwrap())
    };
    let p1 = "- dup\n\t\t- x\n\t\t- a\n";
    let p2 = "- dup\n\t\t\t- x\n\t\t\t- b\n";
    let mut failures = Vec::new();
    for at in ["under", "after", "before"] {
        let (graph, slot, path) = fixture(&format!("{p1}{p2}- target\n"), false);
        let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
        let args = json!({"handle": handle(&read, 0), "to": handle(&read, 1), "at": "after", "rev": read["rev"]});
        verb(&slot, "block.move", args).unwrap();
        let swapped = std::fs::read_to_string(&path).unwrap();
        assert_eq!(swapped, format!("{p2}{p1}- target\n"), "swap");
        let read = verb(&slot, "page.show", json!({"name": "proof"})).unwrap();
        let args = json!({"handle": handle(&read, 0), "to": handle(&read, 2), "at": at, "rev": read["rev"]});
        verb(&slot, "block.move", args).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        let expected = match at {
            "under" => format!("{p1}- target\n\t- dup\n\t\t\t\t- x\n\t\t\t\t- b\n"),
            "after" => format!("{p1}- target\n{p2}"),
            _ => format!("{p1}{p2}- target\n"),
        };
        if after != expected {
            failures.push(format!("{at}: expected {expected:?}, got {after:?}"));
        }
        drop(graph);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
