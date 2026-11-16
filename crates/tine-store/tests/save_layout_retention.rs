//! G6 (og 13a): a save rewrites only the physical lines of blocks it changed.
//! Every case goes through the real `Store::save` entry point and asserts the
//! exact bytes on disk.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::{BlockDto, PageDto};
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

/// Layout the DTO cannot express: every root under a base tab, a depth jump
/// (a child three tabs deep), a whitespace-only continuation line, trailing
/// spaces, and page properties separated by a blank line.
const SOURCE: &str = "title:: Fix\ntags:: a\n\n\t- one\n\t  cont  \n\t  \n\t\t\t- deep\n\t- two\n\t\t- child\n\t- three";

struct Page {
    root: PathBuf,
    store: Store,
    id: PageId,
}

impl Page {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-layout-retention-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::write(root.join("pages/p.md"), source).unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        Page {
            root,
            store,
            id: PageId::from("pages/p.md"),
        }
    }

    /// Apply `edit` to the loaded DTO, save it, and return the disk bytes.
    fn save(&self, kind: EditKind, edit: impl FnOnce(&mut PageDto)) -> String {
        let read = self.store.page(&self.id).unwrap();
        let mut doc = read.doc;
        edit(&mut doc);
        let outcome = self
            .store
            .save(kind, &self.id, SaveBase::Existing(read.rev), &doc);
        assert!(
            matches!(outcome, SaveOutcome::Saved(_) | SaveOutcome::Unchanged(_)),
            "{outcome:?}"
        );
        fs::read_to_string(self.root.join("pages/p.md")).unwrap()
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        self.store.close();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn block(raw: &str) -> BlockDto {
    BlockDto {
        raw: raw.into(),
        ..Default::default()
    }
}

fn saved(source: &str, kind: EditKind, edit: impl FnOnce(&mut PageDto)) -> String {
    Page::new(source).save(kind, edit)
}

#[test]
fn eof_blank_lines_do_not_grow_on_repeated_saves() {
    let _integration_guard = crate::integration_lock();
    // F1: n trailing newlines became 2n-1 on every whole-page save.
    for source in ["- a\n- b\n\n\n", "- a\n-\n\n", "- a\n- b\n- c\n\n\n"] {
        let page = Page::new(source);
        for _ in 0..4 {
            assert_eq!(page.save(EditKind::ReplacePage, |_| {}), source);
        }
        let inserted = page.save(EditKind::InsertBlocks, |doc| {
            doc.blocks.insert(0, block("new"));
        });
        assert_eq!(inserted, format!("- new\n{source}"));
        let deleted = page.save(EditKind::DeleteBlocks, |doc| {
            doc.blocks.remove(0);
        });
        assert_eq!(deleted, source);
    }
}

#[test]
fn eof_blank_lines_survive_whole_page_serialization() {
    let _integration_guard = crate::integration_lock();
    // The fallback serializer must not double them either.
    use tine_core::doc;
    for source in ["- a\n- b\n\n\n", "- a\n-\n\n", "x:: y\n\n- a\n\n\n\n"] {
        let parsed = doc::parse(source);
        let opts = doc::SerializeOpts::detect(Some(source));
        assert_eq!(doc::serialize_with(&parsed, &opts), source);
    }
}

#[test]
fn structural_saves_keep_untouched_block_bytes() {
    let _integration_guard = crate::integration_lock();
    let insert = saved(SOURCE, EditKind::InsertBlocks, |doc| {
        doc.blocks.insert(1, block("new"));
    });
    assert_eq!(insert, SOURCE.replace("\t- two", "\t- new\n\t- two"));

    let paste = saved(SOURCE, EditKind::InsertBlocks, |doc| {
        doc.blocks[0].children.push(block("p1"));
        doc.blocks[0].children.push(block("p2"));
    });
    assert_eq!(
        paste,
        SOURCE.replace("\t\t\t- deep\n", "\t\t\t- deep\n\t\t\t- p1\n\t\t\t- p2\n")
    );

    let delete = saved(SOURCE, EditKind::DeleteBlocks, |doc| {
        doc.blocks.remove(1);
    });
    assert_eq!(delete, SOURCE.replace("\t- two\n\t\t- child\n", ""));

    let moved = saved(SOURCE, EditKind::MoveBlocks, |doc| {
        doc.blocks.swap(1, 2);
    });
    assert_eq!(
        moved,
        SOURCE.replace(
            "\t- two\n\t\t- child\n\t- three",
            "\t- three\n\t- two\n\t\t- child"
        )
    );
}

#[test]
fn insert_after_unclosed_fence_survives_save_and_reopen() {
    let _integration_guard = crate::integration_lock();
    let page = Page::new("- open\n  ```js\n  code\n");
    let written = page.save(EditKind::InsertBlocks, |doc| {
        doc.blocks.push(block("after"));
    });
    assert_eq!(written, "- open\n  ```js\n  code\n- after\n");
    let reopened_store = Store::open(&page.root, Default::default()).unwrap().0;
    let reopened = reopened_store.page(&page.id).unwrap().doc;
    assert_eq!(reopened.blocks.len(), 2);
    assert_eq!(reopened.blocks[1].raw, "after");
    reopened_store.close();
}

#[test]
fn indent_and_outdent_rebase_only_the_moved_subtree() {
    let _integration_guard = crate::integration_lock();
    let indented = saved(SOURCE, EditKind::MoveBlocks, |doc| {
        let two = doc.blocks.remove(1);
        doc.blocks[0].children.push(two);
    });
    // `two` joins `deep` at its sibling's column; `child` stays deeper.
    assert_eq!(
        indented,
        SOURCE.replace("\t- two\n\t\t- child", "\t\t\t- two\n\t\t\t\t- child")
    );

    let outdented = saved(SOURCE, EditKind::MoveBlocks, |doc| {
        let child = doc.blocks[1].children.remove(0);
        doc.blocks.insert(2, child);
    });
    assert_eq!(outdented, SOURCE.replace("\t\t- child", "\t- child"));
}

#[test]
fn reindenting_a_parent_carries_its_original_descendants() {
    let _integration_guard = crate::integration_lock();
    for unit in ["\t", "  "] {
        for newline in ["\n", "\r\n"] {
            let source = "- source\n\t- parent\n\t\t- moving\n\t\t\t- child\n\t\t\t  status:: ready\n\t\t\t  body\n\t\t\t\t- grandchild\n- target\n\t- existing\n- tail\n"
                .replace('\t', unit).replace('\n', newline);
            for kind in [EditKind::MoveBlocks, EditKind::InsertBlocks] {
                let outdented = saved(&source, kind, |doc| {
                    let moving = doc.blocks[0].children[0].children.remove(0);
                    doc.blocks[1].children.push(moving);
                });
                let expected = "- source\n\t- parent\n- target\n\t- existing\n\t- moving\n\t\t- child\n\t\t  status:: ready\n\t\t  body\n\t\t\t- grandchild\n- tail\n"
                    .replace('\t', unit).replace('\n', newline);
                assert_eq!(outdented, expected);
            }

            let pasted = saved(&source, EditKind::InsertBlocks, |doc| {
                let moving = doc.blocks[0].children[0].children[0].clone();
                doc.blocks[1].children.push(moving);
            });
            let copy =
                "\t- moving\n\t\t- child\n\t\t  status:: ready\n\t\t  body\n\t\t\t- grandchild\n"
                    .replace('\t', unit)
                    .replace('\n', newline);
            assert_eq!(pasted, source.replace("- tail", &format!("{copy}- tail")));

            // Existing depth jumps inside an indented subtree move with it.
            let source =
                "- target\n\t- existing\n- moving\n\t\t- child\n\t\t\t- grandchild\n- tail\n"
                    .replace('\t', unit)
                    .replace('\n', newline);
            let indented = saved(&source, EditKind::MoveBlocks, |doc| {
                let moving = doc.blocks.remove(1);
                doc.blocks[0].children.push(moving);
            });
            let expected =
                "- target\n\t- existing\n\t- moving\n\t\t\t- child\n\t\t\t\t- grandchild\n- tail\n"
                    .replace('\t', unit)
                    .replace('\n', newline);
            assert_eq!(indented, expected);
        }
    }
}

#[test]
fn reindenting_a_parent_does_not_shift_a_newly_adopted_child() {
    let _integration_guard = crate::integration_lock();
    let source = "- target\n\t- existing\n- moving\n\t- original\n- donor\n\t\t- adopted\n- tail\n";
    let written = saved(source, EditKind::MoveBlocks, |doc| {
        let adopted = doc.blocks[2].children.remove(0);
        let mut moving = doc.blocks.remove(1);
        moving.children.push(adopted);
        doc.blocks[0].children.push(moving);
    });
    assert_eq!(
        written,
        "- target\n\t- existing\n\t- moving\n\t\t- original\n\t\t- adopted\n- donor\n- tail\n"
    );
}

/// Review F1 of #960: equal text is not identity. Two parents with the same
/// text (prose, an empty bullet, a bulleted or an unbulleted heading) and
/// differently indented children; the first moves under, after or before
/// `target`. The untouched second parent's child keeps its bytes, and the moved
/// child keeps its depth relative to its parent.
#[test]
fn a_repeated_parent_text_keeps_each_parents_children() {
    let _integration_guard = crate::integration_lock();
    let mut failures = Vec::new();
    let mut cases = 0;
    let shapes = [
        ("- dup", "dup", "- dup"),
        ("- ", "", "- "),
        ("- # dup", "# dup", "- # dup"),
        ("# dup", "# dup", "- # dup"),
    ];
    for (line, raw, bulleted) in shapes {
        for unit in ["\t", "  ", "\x0c", " \t"] {
            for newline in ["\n", "\r\n", "\r"] {
                let u = |n: usize| unit.repeat(n);
                let source = format!(
                    "{line}\n{}- childA\n{line}\n{}- childB\n- target\n",
                    u(2),
                    u(3)
                )
                .replace('\n', newline);
                let kept = format!("{line}\n{}- childB\n", u(3));
                for at in ["under", "after", "before"] {
                    let written = saved(&source, EditKind::MoveBlocks, |doc| {
                        assert_eq!(doc.blocks[0].raw, raw);
                        let moving = doc.blocks.remove(0);
                        match at {
                            "under" => doc.blocks[1].children.push(moving),
                            "after" => doc.blocks.push(moving),
                            _ => doc.blocks.insert(1, moving),
                        }
                    });
                    // `target` has no children, so the moved parent takes the
                    // page's own step below it; its child keeps its offset.
                    let step = if unit == "  " { "  " } else { "\t" };
                    let expected = match at {
                        "under" => {
                            format!("{kept}- target\n{step}{bulleted}\n{step}{}- childA\n", u(2))
                        }
                        "after" => format!("{kept}- target\n{line}\n{}- childA\n", u(2)),
                        _ => format!("{kept}{line}\n{}- childA\n- target\n", u(2)),
                    }
                    .replace('\n', newline);
                    cases += 1;
                    if written != expected {
                        failures.push(format!("{line:?} {unit:?} {newline:?} {at}: expected {expected:?}, got {written:?}"));
                    }
                }
            }
        }
    }
    assert_eq!(cases, 144);
    assert!(
        failures.is_empty(),
        "{} of {cases} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Review F1 of #960, insertion: a new subtree with an existing parent's text
/// lands before it while that parent gains a property (as `tine add --before`
/// does). The existing parent's child keeps its bytes; add-under and add-after
/// are controls.
#[test]
fn a_new_subtree_with_a_parents_text_does_not_take_its_children() {
    let _integration_guard = crate::integration_lock();
    let mut failures = Vec::new();
    let mut cases = 0;
    for unit in ["\t", "  ", "\x0c", " \t"] {
        for newline in ["\n", "\r\n", "\r"] {
            let u = |n: usize| unit.repeat(n);
            let source = format!(
                "- dup\n{}- childA\n- dup\n{}- childB\n- target\n",
                u(2),
                u(3)
            )
            .replace('\n', newline);
            for at in ["before", "after", "under"] {
                let written = saved(&source, EditKind::InsertBlocks, |doc| {
                    let mut new = block("dup");
                    new.children.push(block("new-child"));
                    doc.blocks[0]
                        .raw
                        .push_str("\nid:: 6f0c1c1e-0000-4000-8000-000000000001");
                    match at {
                        "before" => doc.blocks.insert(0, new),
                        "after" => doc.blocks.insert(1, new),
                        _ => doc.blocks[0].children.push(new),
                    }
                });
                // The new subtree is rendered in the page's own step.
                let step = if unit == "  " { "  " } else { "\t" };
                let original = format!(
                    "- dup\n  id:: 6f0c1c1e-0000-4000-8000-000000000001\n{}- childA\n",
                    u(2)
                );
                let rest = format!("- dup\n{}- childB\n- target\n", u(3));
                let expected = match at {
                    "before" => format!("- dup\n{step}- new-child\n{original}{rest}"),
                    "after" => format!("{original}- dup\n{step}- new-child\n{rest}"),
                    _ => format!("{original}{}- dup\n{}{step}- new-child\n{rest}", u(2), u(2)),
                }
                .replace('\n', newline);
                cases += 1;
                if written != expected {
                    failures.push(format!(
                        "{unit:?} {newline:?} {at}: expected {expected:?}, got {written:?}"
                    ));
                }
            }
        }
    }
    assert_eq!(cases, 36);
    assert!(
        failures.is_empty(),
        "{} of {cases} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Review 293fa of #960: equal text is not identity, for children either. Two
/// parents share their text and their children repeat: identical children,
/// partly repeated children, repeated children with distinct grandchildren.
/// Moving the first parent (under, after, before `target`) or adding a new
/// subtree with the first parent's texts (under, before, after it) leaves every
/// existing block it does not move byte-identical; a moved block's descendants
/// keep their offsets from it. A new subtree is rendered in the page's step.
#[test]
fn repeated_children_keep_each_parents_bytes() {
    let _integration_guard = crate::integration_lock();
    let mut failures = Vec::new();
    let mut cases = 0;
    let shapes = [
        ("- dup", "dup", "- dup"),
        ("- ", "", "- "),
        ("- # dup", "# dup", "- # dup"),
        ("# dup", "# dup", "- # dup"),
    ];
    // Each family: the first and second parent's descendants as
    // (indent units, text, depth below the parent).
    let families: [(&str, &[(usize, &str, usize)], &[(usize, &str, usize)]); 3] = [
        ("identical", &[(2, "x", 1)], &[(3, "x", 1)]),
        (
            "partly-repeated",
            &[(2, "x", 1), (2, "a", 1)],
            &[(3, "x", 1), (3, "b", 1)],
        ),
        (
            "distinct-grandchildren",
            &[(2, "x", 1), (3, "g1", 2)],
            &[(3, "x", 1), (5, "g2", 2)],
        ),
    ];
    for (line, raw, bulleted) in shapes {
        for unit in ["\t", "  ", " \t"] {
            let u = |n: usize| unit.repeat(n);
            let step = if unit == "  " { "  " } else { "\t" };
            for (family, first, second) in families {
                let lines = |rows: &[(usize, &str, usize)], base: &str| -> String {
                    rows.iter()
                        .map(|(k, t, _)| format!("{base}{}- {t}\n", u(*k)))
                        .collect()
                };
                let p1 = format!("{line}\n{}", lines(first, ""));
                let p2 = format!("{line}\n{}", lines(second, ""));
                // A new copy of the first parent's subtree, in the page's step.
                let fresh = |base: &str| -> String {
                    let head = format!("{base}- {raw}");
                    let body: String = first
                        .iter()
                        .map(|(_, t, d)| format!("{base}{}- {t}\n", step.repeat(*d)))
                        .collect();
                    format!("{}\n{body}", head.trim_end())
                };
                let subtree = || {
                    let mut new = block(raw);
                    for (_, t, d) in first {
                        let mut at = &mut new;
                        for _ in 1..*d {
                            at = at.children.last_mut().unwrap();
                        }
                        at.children.push(block(t));
                    }
                    new
                };
                for newline in ["\n", "\r\n", "\r"] {
                    let source = format!("{p1}{p2}- target\n").replace('\n', newline);
                    let mut check = |name: String, written: String, expected: String| {
                        cases += 1;
                        let expected = expected.replace('\n', newline);
                        if written != expected {
                            failures
                                .push(format!("{name}: expected {expected:?}, got {written:?}"));
                        }
                    };
                    let name = |op: &str| format!("{line:?} {unit:?} {newline:?} {family} {op}");
                    for at in ["under", "after", "before"] {
                        let written = saved(&source, EditKind::MoveBlocks, |doc| {
                            assert_eq!(doc.blocks[0].raw, raw);
                            let moving = doc.blocks.remove(0);
                            match at {
                                "under" => doc.blocks[1].children.push(moving),
                                "after" => doc.blocks.push(moving),
                                _ => doc.blocks.insert(1, moving),
                            }
                        });
                        let expected = match at {
                            "under" => {
                                format!("{p2}- target\n{step}{bulleted}\n{}", lines(first, step))
                            }
                            "after" => format!("{p2}- target\n{p1}"),
                            // Swapping identical subtrees leaves the document
                            // as it was, so the file keeps its bytes (A5).
                            _ if family == "identical" => format!("{p1}{p2}- target\n"),
                            _ => format!("{p2}{p1}- target\n"),
                        };
                        check(name(&format!("move {at}")), written, expected);
                    }
                    for at in ["under", "before", "after"] {
                        let written = saved(&source, EditKind::InsertBlocks, |doc| match at {
                            "under" => doc.blocks[0].children.push(subtree()),
                            "before" => doc.blocks.insert(0, subtree()),
                            _ => doc.blocks.insert(1, subtree()),
                        });
                        // Under the first parent the copy joins its last child's
                        // column; at the root it stands at column zero.
                        let expected = match at {
                            "under" => format!("{p1}{}{p2}- target\n", fresh(&u(2))),
                            "before" => format!("{}{p1}{p2}- target\n", fresh("")),
                            _ => format!("{p1}{}{p2}- target\n", fresh("")),
                        };
                        check(name(&format!("add {at}")), written, expected);
                    }
                }
            }
        }
    }
    assert_eq!(cases, 648);
    assert!(
        failures.is_empty(),
        "{} of {cases} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Identity survives a save: after a move the store keeps the moved blocks'
/// live identities, which a fresh parse of the saved bytes would not assign
/// (it numbers blocks by position). A second move of the same subtree still
/// carries its own children and leaves the other parent's bytes alone.
#[test]
fn a_second_move_after_a_save_keeps_identity() {
    let _integration_guard = crate::integration_lock();
    for unit in ["\t", "  ", " \t"] {
        for newline in ["\n", "\r\n", "\r"] {
            let u = |n: usize| unit.repeat(n);
            let step = if unit == "  " { "  " } else { "\t" };
            let p1 = format!("- dup\n{}- x\n{}- a\n", u(2), u(2));
            let p2 = format!("- dup\n{}- x\n{}- b\n", u(3), u(3));
            let page = Page::new(&format!("{p1}{p2}- target\n").replace('\n', newline));
            let under = page.save(EditKind::MoveBlocks, |doc| {
                let moving = doc.blocks.remove(0);
                doc.blocks[1].children.push(moving);
            });
            let moved = format!("{step}- dup\n{step}{}- x\n{step}{}- a\n", u(2), u(2));
            assert_eq!(
                under,
                format!("{p2}- target\n{moved}").replace('\n', newline),
                "{unit:?} {newline:?} first move"
            );
            // Back out, then before the second parent.
            let back = page.save(EditKind::MoveBlocks, |doc| {
                let moving = doc.blocks[1].children.remove(0);
                doc.blocks.insert(0, moving);
            });
            assert_eq!(
                back,
                format!("{p1}{p2}- target\n").replace('\n', newline),
                "{unit:?} {newline:?} second move"
            );
        }
    }
}

#[test]
fn an_unchanged_parent_keeps_mixed_child_prefixes_verbatim() {
    let _integration_guard = crate::integration_lock();
    for newline in ["\n", "\r\n"] {
        let source = "- root\n\t- parent\n\t\t- inner\n  \t\t- child\n  \t\t\t- grandchild\n  \t\t- second child\n- tail\n".replace('\n', newline);
        let edited = saved(&source, EditKind::SaveBlock, |doc| {
            doc.blocks[0].raw.push('!');
        });
        assert_eq!(edited, source.replacen("root", "root!", 1));
    }
}

#[test]
fn page_property_and_multi_block_edits_keep_other_lines() {
    let _integration_guard = crate::integration_lock();
    let props = saved(SOURCE, EditKind::SaveBlock, |doc| {
        doc.pre_block = Some("title:: Fix\ntags:: a, b".into());
    });
    assert_eq!(props, SOURCE.replace("tags:: a\n", "tags:: a, b\n"));

    let multi = saved(SOURCE, EditKind::SaveBlock, |doc| {
        doc.blocks[1].raw = "two!".into();
        doc.blocks[2].raw = "three!".into();
    });
    assert_eq!(
        multi,
        SOURCE
            .replace("\t- two", "\t- two!")
            .replace("\t- three", "\t- three!")
    );
}

#[test]
fn crlf_pages_keep_untouched_bytes_and_line_endings() {
    let _integration_guard = crate::integration_lock();
    let source = format!("{}\r\n", SOURCE.replace('\n', "\r\n"));
    let edited = saved(&source, EditKind::SaveBlock, |doc| {
        doc.blocks[2].raw = "three!".into();
    });
    assert_eq!(edited, source.replace("\t- three", "\t- three!"));

    let inserted = saved(&source, EditKind::InsertBlocks, |doc| {
        doc.blocks.insert(1, block("new"));
    });
    assert_eq!(inserted, source.replace("\t- two", "\t- new\r\n\t- two"));
}

#[test]
fn first_root_on_a_properties_only_page_gets_one_separator() {
    let _integration_guard = crate::integration_lock();
    for (source, expected) in [
        ("title:: solo\n\n", "title:: solo\n\n- first\n"),
        ("title:: solo\n", "title:: solo\n\n- first\n"),
        ("title:: solo", "title:: solo\n\n- first"),
    ] {
        let inserted = saved(source, EditKind::InsertBlocks, |doc| {
            doc.blocks.push(block("first"));
        });
        assert_eq!(inserted, expected, "{source:?}");
    }
}

/// Reopen the page and return its block tree as `(depth, raw)` in pre-order,
/// so each splice is checked against what the outline parser reads back.
fn reread(page: &Page) -> Vec<(usize, String)> {
    fn walk(blocks: &[BlockDto], depth: usize, out: &mut Vec<(usize, String)>) {
        for b in blocks {
            out.push((depth, b.raw.clone()));
            walk(&b.children, depth + 1, out);
        }
    }
    let store = Store::open(&page.root, Default::default()).unwrap().0;
    let mut out = Vec::new();
    walk(&store.page(&page.id).unwrap().doc.blocks, 0, &mut out);
    store.close();
    out
}

fn tree(rows: &[(usize, &str)]) -> Vec<(usize, String)> {
    rows.iter().map(|(d, r)| (*d, r.to_string())).collect()
}

#[test]
fn unbulleted_headings_keep_their_bytes_when_a_neighbour_is_edited() {
    let _integration_guard = crate::integration_lock();
    // lsdoc (= mldoc) reads each unbulleted ATX heading as its own block; a
    // save that edits another block must not bullet them (master cc9ab56ee).
    let source = "- editable root\n## first section\n### second section\n- trailing root";
    let page = Page::new(source);
    let edited = page.save(EditKind::SaveBlock, |doc| {
        assert_eq!(doc.blocks.len(), 4);
        doc.blocks[0].raw = "edited root".into();
    });
    assert_eq!(edited, source.replace("editable root", "edited root"));
    assert_eq!(
        reread(&page),
        tree(&[
            (0, "edited root"),
            (0, "## first section"),
            (0, "### second section"),
            (0, "trailing root"),
        ])
    );
    let inserted = page.save(EditKind::InsertBlocks, |doc| {
        doc.blocks.insert(2, block("between"));
    });
    assert_eq!(
        inserted,
        "- edited root\n## first section\n- between\n### second section\n- trailing root"
    );
}

#[test]
fn a_leading_heading_stays_unbulleted_through_edits() {
    let _integration_guard = crate::integration_lock();
    // OG writes a leading heading unbulleted with its children one level in
    // (og@6e7afa8 file/core.cljs `transform-content`, `markdown-top-heading?`).
    let source = "# Project\n\t- child one\n\t- child two\n- sibling";
    let page = Page::new(source);
    let child = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].children[1].raw = "child 2".into();
    });
    assert_eq!(child, source.replace("child two", "child 2"));
    let heading = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "# Project!".into();
    });
    assert_eq!(heading, child.replace("# Project", "# Project!"));
    assert_eq!(
        reread(&page),
        tree(&[
            (0, "# Project!"),
            (1, "child one"),
            (1, "child 2"),
            (0, "sibling"),
        ])
    );
}

#[test]
fn a_heading_edited_into_prose_is_written_as_a_bullet() {
    let _integration_guard = crate::integration_lock();
    // Benign-extreme pair of the above: unbulleted prose would be page text,
    // so the block must be bulleted to stay a block.
    let page = Page::new("# Project\n\t- child\n- sibling");
    let written = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "Project".into();
    });
    assert_eq!(written, "- Project\n\t- child\n- sibling");
    assert_eq!(
        reread(&page),
        tree(&[(0, "Project"), (1, "child"), (0, "sibling")])
    );
}

#[test]
fn moving_an_unbulleted_heading_keeps_the_tree() {
    let _integration_guard = crate::integration_lock();
    let page = Page::new("- a\n## h\n- b");
    let written = page.save(EditKind::MoveBlocks, |doc| {
        let h = doc.blocks.remove(1);
        doc.blocks[0].children.push(h);
    });
    assert_eq!(written, "- a\n\t- ## h\n- b");
    assert_eq!(reread(&page), tree(&[(0, "a"), (1, "## h"), (0, "b")]));
}

#[test]
fn every_dash_form_lsdoc_accepts_keeps_its_bytes() {
    let _integration_guard = crate::integration_lock();
    let source = "-\tx\n- \n-\n-  y\n- z";
    let page = Page::new(source);
    let edited = page.save(EditKind::SaveBlock, |doc| {
        doc.blocks[4].raw = "z!".into();
    });
    assert_eq!(edited, format!("{source}!"));
}

#[test]
fn an_edited_block_keeps_its_untouched_whitespace_only_lines() {
    let _integration_guard = crate::integration_lock();
    // og T5: a whitespace-only continuation line parses to an empty raw line,
    // and an edit elsewhere in the block wrote it empty (bytes lost).
    let edited = saved(SOURCE, EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = doc.blocks[0].raw.replacen("one", "ONE", 1);
    });
    assert_eq!(edited, SOURCE.replacen("one", "ONE", 1));
    // Head and tail both survive a line inserted between them; CRLF keeps its terminator.
    let source = "- a\r\n  \r\n  b\r\n   \r\n  c\r\n- d\r\n";
    let edited = saved(source, EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = doc.blocks[0].raw.replacen("b", "b\nnew", 1);
    });
    assert_eq!(edited, "- a\r\n  \r\n  b\r\n  new\r\n   \r\n  c\r\n- d\r\n");
    // A continuation indented less than the bullet's content column (its raw
    // text carries no trace of that) keeps its bytes too.
    let edited = saved("\t- a\n\tb\n", EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "A\nb".into();
    });
    assert_eq!(edited, "\t- A\n\tb\n");
    // A blank line the user typed is new, so it is written empty.
    let edited = saved("- a\n  b", EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "a\n\nb".into();
    });
    assert_eq!(edited, "- a\n\n  b");
}

#[test]
fn a_mixed_ending_page_keeps_each_untouched_terminator() {
    let _integration_guard = crate::integration_lock();
    // GH #110 fidelity gate: once the only CRLF line is deleted, the kept LF
    // lines were rewritten to CRLF (the whole-page convention was applied to
    // retained bytes that carried no `\r`).
    let deleted = saved("- a\r\n- b\n- c\n", EditKind::DeleteBlocks, |doc| {
        doc.blocks.remove(0);
    });
    assert_eq!(deleted, "- b\n- c\n");
    // A rendered line still takes the file's convention.
    let edited = saved("- a\r\n- b\n", EditKind::SaveBlock, |doc| {
        doc.blocks[0].raw = "A".into();
    });
    assert_eq!(edited, "- A\r\n- b\n");
}

#[test]
fn form_feed_layout_keeps_its_bytes() {
    let _integration_guard = crate::integration_lock();
    // GH #110 fidelity gate: lsdoc reads a form feed as layout whitespace, so
    // `\f- b` is a child of `a`; retention did not, declined, and the whole
    // page was re-serialized (`\f` became a tab) for an edit to `c`.
    let source = "- a\n\x0c- b\n- c\n";
    let edited = saved(source, EditKind::SaveBlock, |doc| {
        doc.blocks[1].raw = "c!".into();
    });
    assert_eq!(edited, "- a\n\x0c- b\n- c!\n");
}
