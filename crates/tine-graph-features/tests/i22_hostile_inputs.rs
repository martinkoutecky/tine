//! I-22: hostile outside files must never abort the process that opens them.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tine_store::{FileRev, PageId, SaveBase, Store, StoreError};

#[test]
fn bounded_outline_round_trips_unchanged() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("i22-roundtrip-{}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let mut source = String::new();
    for depth in 0..32 {
        source.push_str(&" ".repeat(depth * 2));
        source.push_str("- body\n");
    }
    let path = root.join("pages/Outline.md");
    fs::write(&path, &source).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let id = PageId::from("pages/Outline.md");
    let read = store.page(&id).unwrap();
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &read.doc
        ),
        tine_store::SaveOutcome::Unchanged(_)
    ));
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        source,
        "I-22: in-bounds outline must round-trip unchanged; exemplar render.rs:719"
    );
    store.close();
}

#[test]
fn anonymized_graph_depth_probe_when_requested() {
    let Ok(root) = std::env::var("TINE_I22_ANON_GRAPH") else {
        return;
    };
    let root = PathBuf::from(root);
    let mut directories = vec![root.join("pages"), root.join("journals")];
    let mut checked = 0usize;
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                directories.push(entry.path());
            } else if matches!(
                entry.path().extension().and_then(|ext| ext.to_str()),
                Some("md" | "org")
            ) {
                let source = fs::read_to_string(entry.path()).unwrap();
                assert!(
                    tine_store::parse_input_depth_within_limit(&source),
                    "anonymized graph page was refused: {}",
                    entry.path().display()
                );
                checked += 1;
            }
        }
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    let unreadable = store.whole_graph().unwrap().unreadable_files().len();
    assert_eq!(unreadable, 0, "anonymized graph has unreadable pages");
    store.close();
    assert!(checked > 0);
    eprintln!("anonymized graph: checked={checked}, refused=0, unreadable=0");
}

#[test]
fn renderer_flattens_tail_without_losing_text() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("i22-flat-{}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let mut source = String::new();
    for depth in 0..120 {
        source.push_str(&" ".repeat(depth * 2));
        source.push_str(if depth == 119 {
            "- LAST-CHILD\n"
        } else {
            "- parent\n"
        });
    }
    fs::write(root.join("pages/Outline.md"), source).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let html = tine_graph_features::print::page_print_html(&store, "Outline", Default::default())
        .unwrap()
        .unwrap();
    assert!(
        html.contains("LAST-CHILD"),
        "I-22: renderer depth bound must flatten rather than drop text; exemplar render.rs:1348"
    );
    store.close();
}

#[test]
fn hostile_inputs_survive_all_entry_points() {
    for case in [
        "oversize",
        "deep_blocks",
        "deep_inline",
        "deep_edn",
        "deep_callouts",
        "deep_org_quote",
        "deep_org_headlines",
        "deep_lists",
        "wide_callouts",
        "wide_quotes",
        "wide_lists",
        "wide_emphasis",
        "wide_outline",
        "wide_org_headlines",
        "wide_mismatched_inline",
    ] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "hostile_child", "--nocapture"])
            .env("TINE_I22_CASE", case)
            .output()
            .unwrap();
        assert!(output.status.success(), "I-22: hostile {case} must not abort open/save/print/publish/conflict diff; exemplar render.rs:719 sanitize at render. exit={:?}; stderr={}", output.status, String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn hostile_child() {
    let Ok(case) = std::env::var("TINE_I22_CASE") else {
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("i22-hostile-{}-{case}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    let name = if matches!(
        case.as_str(),
        "deep_org_quote" | "deep_org_headlines" | "wide_org_headlines"
    ) {
        "pages/Hostile.org"
    } else {
        "pages/Hostile.md"
    };
    let page = root.join(name);
    match case.as_str() {
        "oversize" => {
            let file = fs::File::create(&page).unwrap();
            file.set_len(tine_store::model::PARSE_INPUT_MAX_BYTES + 1)
                .unwrap();
        }
        "deep_blocks" => {
            // 100,000 nested Markdown levels require more than 10 GB of indent
            // alone, so the byte cap rejects that case. This smaller fixture
            // exercises the independent nesting limit below the byte cap.
            let mut text = String::new();
            for depth in 0..1500 {
                text.push_str(&" ".repeat(depth * 2));
                text.push_str("- x\n");
            }
            fs::write(&page, text).unwrap();
        }
        "deep_inline" => {
            fs::write(
                &page,
                format!("- {}x{}\n", "[".repeat(100_000), "]".repeat(100_000)),
            )
            .unwrap();
        }
        "deep_edn" => {
            fs::write(&page, "- ordinary\n").unwrap();
            fs::write(
                root.join("assets/sample.edn"),
                format!("{}0{}", "[".repeat(100_000), "]".repeat(100_000)),
            )
            .unwrap();
        }
        "deep_callouts" => {
            let depth = std::env::var("TINE_I22_DEPTH")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(20_000);
            let mut text = String::from("- root\n");
            for index in 0..depth {
                text.push_str(&format!("  #+BEGIN_a{index}\n"));
            }
            text.push_str("  nested content\n");
            for index in (0..depth).rev() {
                text.push_str(&format!("  #+END_a{index}\n"));
            }
            fs::write(&page, text).unwrap();
        }
        "deep_org_quote" => {
            fs::write(&page, format!("* root\n{}x\n", ">".repeat(20_000))).unwrap();
        }
        "deep_org_headlines" => {
            let mut text = String::new();
            for depth in 1..=2_000 {
                text.push_str(&"*".repeat(depth));
                text.push_str("\n");
            }
            fs::write(&page, text).unwrap();
        }
        "deep_lists" => {
            let mut text = String::from("- root\n");
            for depth in 0..2_000 {
                text.push_str(&" ".repeat(depth + 2));
                text.push_str("+ item\n");
            }
            fs::write(&page, text).unwrap();
        }
        "wide_callouts" => {
            let mut text = String::from("- root\n");
            for index in 0..2_000 {
                text.push_str(&format!(
                    "  #+BEGIN_a{index}\n  content\n  #+END_a{index}\n"
                ));
            }
            fs::write(&page, text).unwrap();
        }
        "wide_quotes" => {
            fs::write(&page, format!("- root\n{}", "  > content\n".repeat(2_000))).unwrap();
        }
        "wide_lists" => {
            fs::write(&page, format!("- root\n{}", "  + item\n".repeat(2_000))).unwrap();
        }
        "wide_emphasis" => {
            fs::write(
                &page,
                format!("- {}x{}\n", "*".repeat(20_000), "*".repeat(20_000)),
            )
            .unwrap();
        }
        "wide_outline" => {
            let mut text = String::new();
            for depth in 0..128 {
                text.push_str(&" ".repeat(depth));
                text.push_str("- item\n");
            }
            fs::write(&page, text).unwrap();
        }
        "wide_org_headlines" => {
            let mut text = String::new();
            for depth in 1..=128 {
                text.push_str(&"*".repeat(depth));
                text.push_str(" item\n");
            }
            fs::write(&page, text).unwrap();
        }
        "wide_mismatched_inline" => {
            fs::write(
                &page,
                format!("- {}({}\n", "[".repeat(5_000), "]".repeat(5_000)),
            )
            .unwrap();
        }
        _ => unreachable!(),
    }
    fs::write(
        root.join("pages/Hostile.sync-conflict-20260926-120000-ABCDEFG.md"),
        "- conflicting\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    if case != "deep_edn" && !case.starts_with("wide_") {
        assert!(
            view.unreadable_files()
                .iter()
                .any(|(id, _)| id.as_str() == name),
            "I-22: hostile page must be listed unreadable; exemplar render.rs:719"
        );
    }
    let id = PageId::from(name);
    let page_result = store.page(&id);
    if case.starts_with("wide_") {
        assert!(
            page_result.is_ok(),
            "I-22: broad, shallow content remains readable"
        );
    }
    if case == "oversize" {
        assert!(
            matches!(page_result, Err(StoreError::TooLarge { limit, .. }) if limit == tine_store::model::PARSE_INPUT_MAX_BYTES),
            "I-22: oversize page must return TooLarge; exemplar render.rs:719"
        );
    }
    if let Ok(read) = page_result {
        let _ = store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &read.doc,
        );
    } else {
        let _ = store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(FileRev::from("missing".to_string())),
            &tine_core::model::PageDto {
                name: "Hostile".into(),
                title: "Hostile".into(),
                kind: tine_core::model::PageKind::Page,
                format: tine_core::model::Format::Md,
                blocks: Vec::new(),
                pre_block: None,
                rev: None,
                read_only: false,
                guide: false,
            },
        );
    }
    let _ = tine_graph_features::print::page_print_html(&store, "Hostile", Default::default());
    let _ = tine_graph_features::publish::publish_html(&store);
    let _ = tine_graph_features::conflicts::sync_conflict_diff(
        &store,
        "pages/Hostile.md",
        "pages/Hostile.sync-conflict-20260926-120000-ABCDEFG.md",
    );
    if case == "deep_edn" {
        let _ = tine_graph_features::pdf::read_highlights(&store, "sample.pdf");
    }
    store.close();
}

/// I-4 + I-22: the benign extreme of the nesting cap. A page exactly at the
/// 128-level admission cap, with realistic multi-line, referencing content,
/// must open, edit, save byte-exactly, reopen, diff against a conflict copy,
/// print, publish and answer queries, on the 2 MiB stack a Tauri command
/// worker runs on. Every recursive consumer is bounded by that cap.
#[test]
fn benign_page_at_depth_cap_round_trips_on_a_command_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(benign_page_at_depth_cap)
        .unwrap()
        .join()
        .unwrap();
}

fn at_cap_outline(leaf: &str) -> String {
    let mut text = String::from("title:: Deep\n\n");
    for depth in 0..128 {
        let indent = "  ".repeat(depth);
        let body = if depth == 127 {
            leaf.to_owned()
        } else {
            format!("level {depth} [[Tag]] with **bold** and ((not a ref))")
        };
        text.push_str(&format!(
            "{indent}- {body}\n{indent}  second line {depth}\n"
        ));
        if depth == 127 {
            text.push_str(&format!(
                "{indent}  #+BEGIN_NOTE\n{indent}  > quoted\n{indent}  #+END_NOTE\n"
            ));
        }
        if depth % 97 == 0 {
            text.push_str(&format!("{indent}  note:: n{depth}\n"));
        }
    }
    text
}

#[test]
fn over_cap_page_is_visible_as_unreadable_with_named_reason() {
    let root = std::env::temp_dir().join(format!("tine-i22-too-deep-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    let mut source = String::new();
    for depth in 0..129 {
        source.push_str(&format!("{}- level {depth}\n", "  ".repeat(depth)));
    }
    fs::write(root.join("pages/TooDeep.md"), source).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(store.page(&PageId::from("pages/TooDeep.md")).is_err());
    let reason = format!("{:?}", store.whole_graph().unwrap().unreadable_files());
    assert!(
        reason.contains("outline nesting exceeds 128 levels"),
        "{reason}"
    );
    store.close();
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

fn benign_page_at_depth_cap() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("i22-at-cap-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let source = at_cap_outline("LEAF needle");
    assert!(tine_store::parse_input_depth_within_limit(&source));
    let path = root.join("pages/Deep.md");
    fs::write(&path, &source).unwrap();
    fs::write(
        root.join("pages/Deep.sync-conflict-20260928-120000-ABCDEFG.md"),
        at_cap_outline("LEAF other"),
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
    let id = PageId::from("pages/Deep.md");
    let read = store.page(&id).unwrap();
    let mut doc = read.doc.clone();
    let mut depth = 1;
    let mut leaf = &mut doc.blocks[0];
    while !leaf.children.is_empty() {
        leaf = &mut leaf.children[0];
        depth += 1;
    }
    assert_eq!(depth, 128, "the page opens with all 128 levels");
    leaf.raw = leaf.raw.replace("LEAF needle", "LEAF edited");
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &doc
        ),
        tine_store::SaveOutcome::Saved(_)
    ));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        at_cap_outline("LEAF edited"),
        "I-4: an edit at the cap changes only the edited block"
    );
    let diff = tine_graph_features::conflicts::sync_conflict_diff(
        &store,
        "pages/Deep.md",
        "pages/Deep.sync-conflict-20260928-120000-ABCDEFG.md",
    )
    .unwrap();
    assert!(diff.is_some(), "merge review opens at the cap");
    let html = tine_graph_features::print::page_print_html(&store, "Deep", Default::default())
        .unwrap()
        .unwrap();
    assert!(html.contains("LEAF edited"));
    tine_graph_features::publish::publish_html(&store).unwrap();
    let whole = store.whole_graph().unwrap();
    whole
        .query("(and [[Tag]] \"level\")", tine_store::QueryDialect::Simple)
        .unwrap();
    whole.backlinks("tag").unwrap();
    drop(whole);
    store.close();
    let reopened = Store::open(&root, Default::default()).unwrap().0;
    assert!(reopened
        .whole_graph()
        .unwrap()
        .unreadable_files()
        .is_empty());
    reopened.page(&id).unwrap();
    reopened.close();
    let _ = fs::remove_dir_all(&root);
}

fn at_cap_org(leaf: &str) -> String {
    let mut text = String::new();
    for depth in 1..=128 {
        let body = if depth == 128 {
            leaf.to_owned()
        } else {
            format!("level {depth} [[Tag]]")
        };
        text.push_str(&format!("{} {body}\nbody {depth}\n", "*".repeat(depth)));
        if depth == 128 {
            text.push_str("#+BEGIN_NOTE\n> quoted\n#+END_NOTE\n");
        }
    }
    text
}

/// The Org twin of the benign extreme: 128 headline levels open, edit and
/// save byte-exactly on a command-worker stack.
#[test]
fn benign_org_page_at_depth_cap_round_trips_on_a_command_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target")
                .join(format!("i22-at-cap-org-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("pages")).unwrap();
            fs::create_dir_all(root.join("journals")).unwrap();
            let path = root.join("pages/DeepOrg.org");
            fs::write(&path, at_cap_org("LEAF needle")).unwrap();
            let store = Store::open(&root, Default::default()).unwrap().0;
            assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
            let id = PageId::from("pages/DeepOrg.org");
            let read = store.page(&id).unwrap();
            let mut doc = read.doc.clone();
            let mut leaf = &mut doc.blocks[0];
            let mut depth = 1;
            while !leaf.children.is_empty() {
                leaf = &mut leaf.children[0];
                depth += 1;
            }
            assert_eq!(depth, 128, "all 128 Org headline levels open");
            leaf.raw = leaf.raw.replace("LEAF needle", "LEAF edited");
            assert!(matches!(
                store.save(
                    tine_store::EditKind::ReplacePage,
                    &id,
                    SaveBase::Existing(read.rev),
                    &doc
                ),
                tine_store::SaveOutcome::Saved(_)
            ));
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                at_cap_org("LEAF edited")
            );
            store.close();
            let _ = fs::remove_dir_all(&root);
        })
        .unwrap()
        .join()
        .unwrap();
}
