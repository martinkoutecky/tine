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
        store.save(&id, SaveBase::Existing(read.rev), &read.doc),
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
fn renderer_flattens_tail_without_losing_text() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("i22-flat-{}", std::process::id()));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    let mut source = String::new();
    for depth in 0..180 {
        source.push_str(&" ".repeat(depth * 2));
        source.push_str(if depth == 179 {
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
    for case in ["oversize", "deep_blocks", "deep_inline", "deep_edn"] {
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
    let page = root.join("pages/Hostile.md");
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
        _ => unreachable!(),
    }
    fs::write(
        root.join("pages/Hostile.sync-conflict-20260926.md"),
        "- conflicting\n",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let view = store.whole_graph().unwrap();
    if case != "deep_edn" {
        assert!(
            view.unreadable_files()
                .iter()
                .any(|(id, _)| id.as_str() == "pages/Hostile.md"),
            "I-22: hostile page must be listed unreadable; exemplar render.rs:719"
        );
    }
    let id = PageId::from("pages/Hostile.md");
    let page_result = store.page(&id);
    if case == "oversize" {
        assert!(
            matches!(page_result, Err(StoreError::TooLarge { limit, .. }) if limit == tine_store::model::PARSE_INPUT_MAX_BYTES),
            "I-22: oversize page must return TooLarge; exemplar render.rs:719"
        );
    }
    if let Ok(read) = page_result {
        let _ = store.save(&id, SaveBase::Existing(read.rev), &read.doc);
    } else {
        let _ = store.save(
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
        "pages/Hostile.sync-conflict-20260926.md",
    );
    if case == "deep_edn" {
        let _ = tine_graph_features::pdf::read_highlights(&store, "sample.pdf");
    }
    store.close();
}
