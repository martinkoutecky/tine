//! Sibling families from the second move/swap review: no direct child survives
//! unchanged, but descendants still own their original physical lines.
use super::*;

fn clear_ids(blocks: &mut [DocBlock]) {
    for block in blocks {
        block.uuid = BlockId::default();
        clear_ids(&mut block.children);
    }
}

#[test]
fn ambiguous_six_thousand_block_matching_has_a_timing_budget() {
    let tail: String = (0..5993).map(|i| format!("- ballast {i}\n")).collect();
    let source = format!("- dup\n\t\t\t- x\n\t\t\t- b\n- dup\n\t\t- x\n\t\t- a\n- target\n{tail}");
    let mut old = super::super::parse_doc(std::path::Path::new("p.md"), &source);
    clear_ids(&mut old.roots);
    let lines: Vec<_> = source.lines().collect();
    let olds = map_old_blocks(&old, &lines).unwrap();
    assert_eq!(olds.len(), 6000);
    let mut doc = old.clone();
    doc.roots[0].children[0].set_raw("x!");
    doc.roots.insert(0, DocBlock::new("dup"));
    let mut news = Vec::new();
    flatten(&doc.roots, &mut news);
    let start = std::time::Instant::now();
    let matches = match_blocks(&olds, &news);
    let elapsed = start.elapsed();
    eprintln!("ambiguous 6000-block matcher: {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "{elapsed:?}"
    );
    assert_eq!(
        matches.keep[3],
        Some(2),
        "untouched b keeps its physical lines"
    );
}

#[test]
fn inserted_leaves_preserve_edited_containers_and_untouched_descendants() {
    let mut cases = 0;
    let mut failures = Vec::new();
    for header in [
        "- dup",
        "- prose text",
        "-",
        "- ## dup",
        "## dup",
        "### dup",
    ] {
        for unit in ["\t", "  ", "\x0c", " \t"] {
            for newline in ["\n", "\r\n", "\r"] {
                // Flat/nested children, with/without a parent edit: every
                // direct child changes, while the nested grandchild does not.
                for shape in 0..4 {
                    let children = |letter: &str, depth: usize| match shape {
                        0 | 1 => format!(
                            "{}- x\n{}- {letter}\n",
                            unit.repeat(depth),
                            unit.repeat(depth)
                        ),
                        _ => format!(
                            "{}- x\n{}- {letter}\n",
                            unit.repeat(depth),
                            unit.repeat(depth * 2)
                        ),
                    };
                    let first = format!("{header}\n{}", children("a", 2));
                    let second = format!("{header}\n{}", children("b", 3));
                    let source = format!("{second}{first}- target\n").replace('\n', newline);
                    let mut old = super::super::parse_doc(std::path::Path::new("p.md"), &source);
                    clear_ids(&mut old.roots);
                    assert_eq!(old.roots.len(), 3);
                    for before in [true, false] {
                        for same in [true, false] {
                            let mut doc = old.clone();
                            for child in &mut doc.roots[0].children {
                                child.set_raw(format!("{}!", child.raw()));
                            }
                            if shape % 2 == 1 {
                                let raw = format!("{}!", doc.roots[0].raw());
                                doc.roots[0].set_raw(raw);
                            }
                            let leaf_raw = if same { old.roots[0].raw() } else { "inserted" };
                            doc.roots
                                .insert(usize::from(!before), DocBlock::new(leaf_raw));
                            let mut edited = match shape {
                                0 | 1 => {
                                    second.replace("- x\n", "- x!\n").replace("- b\n", "- b!\n")
                                }
                                _ => second.replace("- x\n", "- x!\n"),
                            };
                            if shape % 2 == 1 {
                                let edited_header = if header == "-" {
                                    "- !".into()
                                } else {
                                    format!("{header}!")
                                };
                                edited = edited.replacen(header, &edited_header, 1);
                            }
                            let leaf = if leaf_raw.is_empty() {
                                "-\n".into()
                            } else {
                                format!("- {leaf_raw}\n")
                            };
                            let expected = if before {
                                format!("{leaf}{edited}{first}- target\n")
                            } else {
                                format!("{edited}{leaf}{first}- target\n")
                            }
                            .replace('\n', newline);
                            let written = serialize(&doc, &source, &old, &SerializeOpts::default())
                                .map(|v| v.0);
                            cases += 1;
                            if written.as_deref() != Some(&expected) {
                                failures.push(format!("{header:?} {unit:?} {newline:?} shape={shape} before={before} same={same}: {written:?}"));
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "ownership serializer: {} failures / {cases}",
        failures.len()
    );
    assert_eq!(cases, 1152);
    assert!(
        failures.is_empty(),
        "{} of {cases} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn inserted_leaves_preserve_org_descendant_terminators() {
    use tine_core::org::{parse_org, serialize_org_detect};
    let mut cases = 0;
    let mut failures = Vec::new();
    for parent_too in [true, false] {
        for before in [true, false] {
            for same in [true, false] {
                let first = "* dup\r\n** x\n*** a\r\n";
                let second = "* dup\r\n** x\n*** b\r";
                let source = format!("{second}{first}* target\r\n");
                let mut old = parse_org(&source);
                clear_ids(&mut old.roots);
                let mut doc = old.clone();
                doc.roots[0].children[0].set_raw("x!");
                if parent_too {
                    doc.roots[0].set_raw("dup!");
                }
                let leaf_raw = if same { "dup" } else { "inserted" };
                doc.roots
                    .insert(usize::from(!before), DocBlock::new(leaf_raw));
                let mut edited = second.replace("** x\n", "** x!\n");
                if parent_too {
                    edited = edited.replacen("dup", "dup!", 1);
                }
                let leaf = format!("* {leaf_raw}\r\n");
                let expected = if before {
                    format!("{leaf}{edited}{first}* target\r\n")
                } else {
                    format!("{edited}{leaf}{first}* target\r\n")
                };
                let written = restore_org(
                    serialize_org_detect(&doc, Some(&source)),
                    &doc,
                    &source,
                    &old,
                );
                cases += 1;
                if written != expected {
                    failures.push(format!(
                        "parent={parent_too} before={before} same={same}: {written:?}"
                    ));
                }
            }
        }
    }
    eprintln!("ownership Org: {} failures / {cases}", failures.len());
    assert_eq!(cases, 8);
    assert!(
        failures.is_empty(),
        "{} of {cases} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
