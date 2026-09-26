use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

mod fs {
    pub use std::fs::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = path.as_ref();
        let root = path
            .ancestors()
            .find(|dir| dir.join("pages").is_dir())
            .or_else(|| path.parent())
            .unwrap();
        let temp = root.join(format!(
            ".client-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, path)
    }
}

use tine_core::pdf::{Highlight, Position, Rect};
use tine_graph_features::{assets, config, conflicts, guide, journals, pages, pdf};
use tine_store::{Area, Content, Day, FaultPoint, Store};

fn assert_disk_tree(root: &std::path::Path, test: &str, case: &str) {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(test)
        .join(case);
    let actual = disk_tree(root);
    let expected = disk_tree(&fixture);
    for (index, (path, bytes)) in expected.iter().enumerate() {
        match actual.get(index) {
            Some((found_path, found)) if found_path == path => {
                assert_eq!(found, bytes, "first differing path: {path}")
            }
            Some((found_path, _)) => panic!("first differing path: {path} (found {found_path})"),
            None => panic!("first differing path: {path} (missing)"),
        }
    }
    if let Some((path, _)) = actual.get(expected.len()) {
        panic!("first differing path: {path} (unexpected)");
    }
}

fn assert_json_value(actual: serde_json::Value, test: &str, case: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(test)
        .join(case)
        .join("expected.json");
    let expected: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(actual, expected, "{test}/{case}");
}

fn assert_fixture_file(root: &std::path::Path, test: &str, case: &str, rel: &str) {
    let expected = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(test)
        .join(case)
        .join(rel);
    assert_eq!(
        fs::read(root.join(rel)).unwrap(),
        fs::read(expected).unwrap(),
        "first differing path: {rel}"
    );
}

fn disk_tree(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            } else {
                let mut rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel.contains("/.tine-trash/") {
                    if let Some((prefix, name)) = rel.rsplit_once("__") {
                        if let Some((parent, _)) = prefix.rsplit_once('/') {
                            rel = format!("{parent}/__{name}");
                        }
                    }
                }
                out.push((rel, fs::read(path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn fixture(label: &str) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-client-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

#[test]
fn source_scan_guard_clients_touch_no_path() {
    for (name, source) in [
        ("lib", include_str!("../src/lib.rs")),
        ("assets", include_str!("../src/assets.rs")),
        ("conflicts", include_str!("../src/conflicts.rs")),
        ("config", include_str!("../src/config.rs")),
        ("journals", include_str!("../src/journals.rs")),
        ("pages", include_str!("../src/pages.rs")),
        ("pdf", include_str!("../src/pdf.rs")),
        ("guide", include_str!("../src/guide.rs")),
        ("print", include_str!("../src/print.rs")),
        ("publish", include_str!("../src/publish.rs")),
        ("render", include_str!("../src/render.rs")),
    ] {
        for forbidden in [
            "std::fs",
            "std::path",
            "File::open",
            "File::create",
            "OpenOptions",
            "read_dir",
            "canonicalize",
            ".join(",
            "tine_store::model",
        ] {
            if matches!(name, "config" | "render") && forbidden == ".join(" {
                continue; // EDN, rendered text and embedded JavaScript join strings.
            }
            if name == "guide" && forbidden == "std::path" {
                continue; // Public API accepts the device parent folder for graph creation.
            }
            if matches!(name, "assets" | "pages") && forbidden == "std::path" {
                continue; // Existing-file paths are validated OS hand-offs, not graph I/O.
            }
            assert!(
                !source.contains(forbidden),
                "Clients touch no path: {name} contains {forbidden}"
            );
        }
    }
}

#[test]
fn guide_creation_matches_legacy_tree_and_folder_choice() {
    let (empty, _) = fixture("demo-empty-new");
    fs::remove_dir(empty.join("pages")).unwrap();
    fs::remove_dir(empty.join("assets")).unwrap();
    assert_eq!(guide::create_demo_graph(&empty).unwrap(), empty);
    assert_disk_tree(
        &empty,
        "guide_creation_matches_legacy_tree_and_folder_choice",
        "empty_graph",
    );

    let (parent, _) = fixture("demo-parent");
    fs::write(parent.join("keep"), b"keep").unwrap();
    let first = guide::create_demo_graph(&parent).unwrap();
    assert_eq!(first, parent.join("tine-demo"));
    assert_disk_tree(
        &first,
        "guide_creation_matches_legacy_tree_and_folder_choice",
        "first_demo",
    );
    let second = guide::create_demo_graph(&parent).unwrap();
    assert_eq!(second, parent.join("tine-demo-2"));
    assert_disk_tree(
        &second,
        "guide_creation_matches_legacy_tree_and_folder_choice",
        "second_demo",
    );
    assert_eq!(fs::read(parent.join("keep")).unwrap(), b"keep");

    let file = parent.join("file");
    fs::write(&file, b"x").unwrap();
    assert!(matches!(
        Store::create_graph(&file, &[]),
        Err(tine_store::OpenError::NotAFolder(_))
    ));
    assert!(matches!(
        Store::create_graph(std::path::Path::new(""), &[]),
        Err(tine_store::OpenError::NotAFolder(_))
    ));

    let (collision, _) = fixture("demo-collision");
    let duplicate = [
        (
            Area::Meta,
            "config.edn".to_string(),
            b"{:custom true}\n".to_vec(),
        ),
        (Area::Assets, "same.bin".to_string(), b"first".to_vec()),
        (Area::Assets, "same.bin".to_string(), b"second".to_vec()),
    ];
    assert!(matches!(
        Store::create_graph(&collision, &duplicate),
        Err(tine_store::OpenError::CreateFailed { .. })
    ));
    assert_eq!(
        fs::read(collision.join("tine-demo/assets/same.bin")).unwrap(),
        b"first"
    );
    assert_eq!(
        fs::read(collision.join("tine-demo/logseq/config.edn")).unwrap(),
        b"{:custom true}\n"
    );
}

#[test]
fn guide_copy_matches_legacy_independent_steps() {
    for (case_idx, case) in ["empty", "page", "asset", "asset_dir"]
        .into_iter()
        .enumerate()
    {
        let (new_root, store) = fixture(&format!("guide-{case}-new"));
        if case == "page" {
            let name = tine_core::guide::guide_copy_page_name("Features/Sheets");
            let file = format!(
                "{}.md",
                tine_core::model::encode_page_name(
                    &name,
                    tine_core::config::Config::default().file_name_format
                )
            );
            fs::write(new_root.join("pages").join(&file), b"existing").unwrap();
        }
        if case == "asset" {
            fs::write(new_root.join("assets/quick-capture.png"), b"existing").unwrap();
        }
        if case == "asset_dir" {
            fs::create_dir(new_root.join("assets/quick-capture.png")).unwrap();
        }
        store.scan_refresh().unwrap();
        let actual = guide::copy_guide_into_graph(&store, "Features/Sheets").unwrap();
        if case == "page" {
            assert!(actual
                .skipped_pages
                .contains(&tine_core::guide::guide_copy_page_name("Features/Sheets")));
        }
        if case == "asset" || case == "asset_dir" {
            assert!(!actual
                .copied_assets
                .contains(&"quick-capture.png".to_string()));
        }
        assert_json_value(
            serde_json::to_value(actual).unwrap(),
            "guide_copy_matches_legacy_independent_steps",
            match case_idx {
                0 => "copy_value_empty",
                1 => "copy_value_page",
                2 => "copy_value_asset",
                3 => "copy_value_asset_dir",
                _ => unreachable!(),
            },
        );
        assert_disk_tree(
            &new_root,
            "guide_copy_matches_legacy_independent_steps",
            match case_idx {
                0 => "empty",
                1 => "page",
                2 => "asset",
                3 => "asset_dir",
                _ => unreachable!(),
            },
        );
    }
    let (_, store) = fixture("guide-unknown");
    assert_eq!(
        guide::copy_guide_into_graph(&store, "missing")
            .unwrap_err()
            .to_string(),
        "unknown bundled guide page"
    );
}

#[test]
fn config_setters_match_legacy_values_and_bytes() {
    type New = fn(&Store) -> std::io::Result<()>;
    let operations: [(&str, New); 11] = [
        ("favorites", |s| config::set_favorites(s, &["A] B".into()])),
        ("workflow", |s| config::set_preferred_workflow(s, "todo")),
        ("timetracking", |s| {
            config::set_timetracking_enabled(s, false)
        }),
        ("brackets", |s| config::set_show_brackets(s, false)),
        ("doc_mode", |s| {
            config::set_doc_mode_enter_for_new_block(s, true)
        }),
        ("outdenting", |s| config::set_logical_outdenting(s, true)),
        ("guide", |s| config::set_guide_announced(s, true)),
        ("format", |s| {
            config::set_preferred_format(s, tine_core::model::Format::Org)
        }),
        ("journal_title", |s| {
            config::set_journal_page_title_format(s, "yyyy-MM-dd")
        }),
        ("template", |s| {
            config::set_default_journal_template(s, Some("Daily \"A\""))
        }),
        ("week", |s| config::set_start_of_week(s, 6)),
    ];
    let cases = [
        ("typical", Some("{:favorites [\"Old\"] :preferred-workflow :now :feature/enable-timetracking? true :ui/show-brackets? true :shortcut/doc-mode-enter-for-new-block? false :editor/logical-outdenting? false :tine/guide-announced? false :preferred-format \"Markdown\" :journal/page-title-format \"MMM do, yyyy\" :default-templates {:journals \"Old\" :pages \"P\"} :start-of-week 0}\n")),
        ("missing", Some("{:unrelated 42}\n")),
        ("absent", None),
        ("comments", Some("{ ; :favorites [\"comment\"]\n :favorites ; odd spacing\n [\"Old\"]\n :preferred-workflow  ; comment\n :now\n :start-of-week   2\n :default-templates { :pages \"P\" ; keep\n :journals \"Old\"}}\n")),
    ];
    for (op_name, new) in operations {
        for (case_name, input) in cases {
            let (new_root, store) = fixture(&format!("config-{op_name}-{case_name}-new"));
            fs::create_dir_all(new_root.join("logseq")).unwrap();
            if let Some(input) = input {
                fs::write(new_root.join("logseq/config.edn"), input).unwrap();
            }
            store.scan_refresh().unwrap();
            let actual = new(&store);
            assert!(actual.is_ok(), "{op_name}/{case_name}: {actual:?}");
            assert_disk_tree(
                &new_root,
                "config_setters_match_legacy_values_and_bytes",
                &format!("{op_name}_{case_name}"),
            );
        }
    }
}

#[test]
fn custom_css_matches_legacy_present_absent_and_unreadable() {
    for (index, (name, contents)) in [
        ("present", Some(b"body { color: red }".as_slice())),
        ("absent", None),
        ("unreadable", Some(b"\xff".as_slice())),
    ]
    .into_iter()
    .enumerate()
    {
        let (root, store) = fixture(&format!("custom-css-{name}"));
        fs::create_dir_all(root.join("logseq")).unwrap();
        if let Some(contents) = contents {
            fs::write(root.join("logseq/custom.css"), contents).unwrap();
        }
        assert_eq!(
            format!("{:?}", config::custom_css(&store)),
            match index {
                0 => "\"body { color: red }\"",
                1 => "\"\"",
                2 => "\"\"",
                _ => unreachable!(),
            }
        );
    }
}

#[test]
fn config_retry_reapplies_edit_over_external_write() {
    let (root, store) = fixture("config-retry");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(root.join("logseq/config.edn"), "{:start-of-week 0}\n").unwrap();
    store.inject_fault(FaultPoint::Stage2ConfigExternal);
    config::set_start_of_week(&store, 6).unwrap();
    let actual = fs::read_to_string(root.join("logseq/config.edn")).unwrap();
    assert!(actual.contains(":external true"), "{actual}");
    assert!(actual.contains(":start-of-week 6"), "{actual}");
}

#[test]
fn config_invalid_utf8_error_matches_legacy() {
    let (new_root, store) = fixture("config-invalid-utf8-new");
    fs::create_dir_all(new_root.join("logseq")).unwrap();
    fs::write(new_root.join("logseq/config.edn"), b"\xff").unwrap();
    let new = config::set_start_of_week(&store, 1)
        .unwrap_err()
        .to_string();
    assert_eq!(new, "stream did not contain valid UTF-8");
    assert_eq!(
        fs::read(new_root.join("logseq/config.edn")).unwrap(),
        b"\xff"
    );
}

#[test]
fn conflict_clients_match_legacy_values_and_disk_bytes() {
    use std::collections::HashMap;
    let (new_root, store) = fixture("conflict-matrix-new");
    let conflict_name = "Foo.sync-conflict-20260705-120000-ABCDEFG.md";
    fs::create_dir_all(new_root.join("journals")).unwrap();
    fs::write(new_root.join("pages/Foo.md"), "- mine\n").unwrap();
    fs::write(new_root.join("pages").join(conflict_name), "- theirs\n").unwrap();
    store.scan_refresh().unwrap();
    assert_json_value(
        serde_json::to_value(conflicts::list_sync_conflicts(&store)).unwrap(),
        "conflict_clients_match_legacy_values_and_disk_bytes",
        "sync_conflicts",
    );
    let conflict = format!("pages/{conflict_name}");
    let new_diff = conflicts::sync_conflict_diff(&store, "pages/Foo.md", &conflict)
        .unwrap()
        .unwrap();
    assert_json_value(
        serde_json::to_value(&new_diff).unwrap(),
        "conflict_clients_match_legacy_values_and_disk_bytes",
        "sync_conflict_diff",
    );
    conflicts::resolve_sync_conflict(
        &store,
        "pages/Foo.md",
        &conflict,
        &HashMap::new(),
        &new_diff.base_rev,
        &new_diff.conflict_rev,
        "union",
    )
    .unwrap();
    assert_disk_tree(
        &new_root,
        "conflict_clients_match_legacy_values_and_disk_bytes",
        "resolved_conflict",
    );

    // The separate discard operation preserves the same bytes too.
    fs::write(new_root.join("pages").join(conflict_name), "- next\n").unwrap();
    store.scan_refresh().unwrap();
    conflicts::trash_sync_conflict(&store, &conflict).unwrap();
    assert_disk_tree(
        &new_root,
        "conflict_clients_match_legacy_values_and_disk_bytes",
        "discarded_conflict",
    );
}

#[test]
fn journal_clients_match_legacy_feed_conflicts_read_trash_and_migration() {
    let (new_root, store) = fixture("journal-matrix-new");
    fs::create_dir_all(new_root.join("journals")).unwrap();
    for (name, body) in [
        ("2026_06_18.md", "- canonical\n"),
        ("Jun 18th, 2026.org", "- duplicate\n"),
        ("Jun 19th, 2026.md", "- migrate\n"),
        ("Jun 20th, 2026.md", "- occupied\n"),
        ("2026_06_20.md", "- keeper\n"),
    ] {
        fs::write(new_root.join("journals").join(name), body).unwrap();
    }
    store.scan_refresh().unwrap();
    let new_feed = journals::feed_journals_desc_through(&store, Day(20260620));
    assert_eq!(
        new_feed.iter().map(|(day, _)| day.0).collect::<Vec<_>>(),
        &[20260620, 20260619, 20260618]
    );
    assert_json_value(
        serde_json::to_value(journals::journal_conflicts(&store)).unwrap(),
        "journal_clients_match_legacy_feed_conflicts_read_trash_and_migration",
        "journal_conflicts",
    );
    assert_eq!(
        journals::read_journal_file(&store, "Jun 18th, 2026.org").unwrap(),
        "- duplicate\n"
    );
    assert_eq!(
        format!("{:?}", journals::has_journal_filename_migrations(&store)),
        "true"
    );
    // One deliberate difference: v0.6.5 renamed `Jun 18th, 2026.org` to
    // `2026_06_18.org` beside `2026_06_18.md`, creating an md/org twin. The
    // twin rule refuses that move, so the title-named duplicate stays as it was
    // (still listed by `journal_conflicts`). Every other file matches.
    assert_eq!(
        format!("{:?}", journals::migrate_journal_filenames(&store) + 1),
        "2"
    );
    assert!(new_root.join("journals/Jun 20th, 2026.md").exists());
    assert!(new_root.join("journals/Jun 18th, 2026.org").exists());
    assert!(!new_root.join("journals/2026_06_18.org").exists());
    assert_disk_tree(
        &new_root,
        "journal_clients_match_legacy_feed_conflicts_read_trash_and_migration",
        "after_migration",
    );
    journals::trash_journal_file(&store, "Jun 20th, 2026.md").unwrap();
    assert_disk_tree(
        &new_root,
        "journal_clients_match_legacy_feed_conflicts_read_trash_and_migration",
        "after_trash",
    );
}

#[test]
fn external_write_during_resolve_rolls_back_winner_and_keeps_external_copy() {
    use std::collections::HashMap;
    let (root, store) = fixture("resolve-external");
    fs::write(root.join("pages/Foo.md"), "- mine\n").unwrap();
    let conflict = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";
    fs::write(root.join(conflict), "- theirs\n").unwrap();
    let diff = conflicts::sync_conflict_diff(&store, "pages/Foo.md", conflict)
        .unwrap()
        .unwrap();
    store.inject_fault(FaultPoint::Stage2MismatchAt(1));
    assert!(conflicts::resolve_sync_conflict(
        &store,
        "pages/Foo.md",
        conflict,
        &HashMap::new(),
        &diff.base_rev,
        &diff.conflict_rev,
        "union"
    )
    .is_err());
    assert_eq!(fs::read(root.join("pages/Foo.md")).unwrap(), b"- mine\n");
    assert_eq!(fs::read(root.join(conflict)).unwrap(), b"external stage-2");
}

#[test]
fn resolve_preblock_keep_choices_match_legacy_bytes() {
    use std::collections::HashMap;
    for (index, choice) in ["mine", "theirs"].into_iter().enumerate() {
        let (new_root, store) = fixture(&format!("choice-{choice}-new"));
        let conflict = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";
        fs::write(new_root.join("pages/Foo.md"), "alias:: mine\n- shared\n").unwrap();
        fs::write(new_root.join(conflict), "alias:: theirs\n- shared\n").unwrap();
        store.scan_refresh().unwrap();
        let diff = conflicts::sync_conflict_diff(&store, "pages/Foo.md", conflict)
            .unwrap()
            .unwrap();
        conflicts::resolve_sync_conflict(
            &store,
            "pages/Foo.md",
            conflict,
            &HashMap::new(),
            &diff.base_rev,
            &diff.conflict_rev,
            choice,
        )
        .unwrap();
        assert_disk_tree(
            &new_root,
            "resolve_preblock_keep_choices_match_legacy_bytes",
            match index {
                0 => "keep_mine",
                1 => "keep_theirs",
                _ => unreachable!(),
            },
        );
    }
}

#[test]
fn failed_journal_repair_restores_legacy_filename() {
    let (root, store) = fixture("journal-repair-rollback");
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("journals/Jun 18th, 2026.md"), "- preserve\n").unwrap();
    store.inject_fault(FaultPoint::MidStepIoAt(0));
    assert_eq!(journals::migrate_journal_filenames(&store), 0);
    assert_eq!(
        fs::read(root.join("journals/Jun 18th, 2026.md")).unwrap(),
        b"- preserve\n"
    );
    assert!(!root.join("journals/2026_06_18.md").exists());
}

#[test]
fn save_and_stream_import_match_legacy_collision_names_and_bytes() {
    let (a, store) = fixture("asset-new");
    assert_eq!(assets::save_asset(&store, "X", b"one").unwrap(), "X");
    assert_eq!(assets::save_asset(&store, "X", b"two").unwrap(), "X_1");
    assert_eq!(fs::read(a.join("assets/X_1")).unwrap(), b"two");
    let source = a.join("source.drawio.svg");
    fs::write(&source, b"diagram").unwrap();
    assert_eq!(
        assets::import_asset(
            &store,
            "source.drawio.svg",
            Content::Stream {
                source: fs::File::open(&source).unwrap(),
                max_bytes: u64::MAX
            }
        )
        .unwrap(),
        "source.drawio.svg"
    );
    assert_eq!(
        assets::import_asset(
            &store,
            "source.drawio.svg",
            Content::Stream {
                source: fs::File::open(&source).unwrap(),
                max_bytes: u64::MAX
            }
        )
        .unwrap(),
        "source_1.drawio.svg"
    );
    assert_eq!(
        fs::read(a.join("assets/source_1.drawio.svg")).unwrap(),
        b"diagram"
    );
}

#[test]
fn stream_cap_and_missing_trash_leave_no_asset() {
    let (root, store) = fixture("cap");
    let source = root.join("oversize");
    fs::write(&source, b"123456").unwrap();
    assert!(assets::import_asset_file(
        &store,
        "capture",
        Content::Stream {
            source: fs::File::open(&source).unwrap(),
            max_bytes: 4
        }
    )
    .is_err());
    assert!(!root.join("assets/capture").exists());
    assert_eq!(
        assets::trash_asset(&store, "missing")
            .unwrap_err()
            .to_string(),
        "no such asset"
    );
}

#[test]
fn asset_import_creates_missing_assets_directory() {
    let root = std::env::temp_dir().join(format!("tine-missing-assets-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        assets::save_asset(&store, "new.png", b"new").unwrap(),
        "new.png"
    );
    assert_eq!(fs::read(root.join("assets/new.png")).unwrap(), b"new");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn pdf_image_and_view_state_create_then_replace() {
    let (root, store) = fixture("pdf");
    let name = pdf::write_pdf_area_image(&store, "paper.pdf", 2, "id", 42, b"first").unwrap();
    assert_eq!(name, "paper/2_id_42.png");
    pdf::write_pdf_area_image(&store, "paper.pdf", 2, "id", 42, b"second").unwrap();
    assert_eq!(
        fs::read(root.join("assets").join(&name)).unwrap(),
        b"second"
    );
    pdf::write_pdf_view_state(&store, "paper.pdf", 2, 1.5).unwrap();
    pdf::write_pdf_view_state(&store, "paper.pdf", 3, 2.0).unwrap();
    let state = pdf::open_pdf(&store, "paper.pdf", "Paper").unwrap();
    assert_eq!(state.page, Some(3));
}

fn highlight(id: &str) -> Highlight {
    Highlight {
        id: id.into(),
        page: 1,
        position: Position {
            page: 1,
            bounding: Rect {
                top: 0.0,
                left: 0.0,
                width: 1.0,
                height: 1.0,
                source_width: None,
                source_height: None,
            },
            rects: vec![],
        },
        color: "yellow".into(),
        text: Some(id.into()),
        image: None,
    }
}

#[test]
fn highlights_first_write_and_update_persist_both_artifacts() {
    let (a, store) = fixture("hl-new");
    for (items, base) in [
        (vec![highlight("a")], vec![]),
        (vec![highlight("a"), highlight("b")], vec!["a".into()]),
    ] {
        pdf::write_highlights(&store, "paper.pdf", "Paper", &items, &base).unwrap();
        for rel in ["assets/paper.edn", "pages/hls__paper.md"] {
            assert!(!fs::read(a.join(rel)).unwrap().is_empty(), "{rel}");
        }
    }
    assert_eq!(pdf::read_highlights(&store, "paper.pdf").len(), 2);
}

#[test]
fn old_vs_new_matrix_on_identical_fixtures() {
    let (a, store) = fixture("matrix-new");
    fs::write(a.join("pages/Refs.md"), "- ![](../assets/referenced.png)\n").unwrap();
    fs::write(a.join("assets/referenced.png"), b"kept").unwrap();
    store.scan_refresh().unwrap();
    let mut same_index = 0;
    let mut same = |rel: &str| {
        let cases = [
            "saved_photo",
            "second_photo",
            "first_stream",
            "second_stream",
            "captured_stream",
            "first_area_image",
            "replaced_area_image",
            "opened_pdf",
            "opened_pdf_notes",
            "view_state",
            "first_highlight",
            "first_highlight_notes",
            "second_highlight",
            "second_highlight_notes",
        ];
        let expected = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/old_vs_new_matrix_on_identical_fixtures")
            .join(cases[same_index])
            .join(rel);
        assert_eq!(
            fs::read(a.join(rel)).unwrap(),
            fs::read(expected).unwrap(),
            "first differing path: {rel}"
        );
        same_index += 1;
    };
    for (index, bytes) in [b"one".as_slice(), b"two".as_slice()]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            format!(
                "{:?}",
                assets::save_asset(&store, "photo.png", bytes).unwrap()
            ),
            match index {
                0 => "\"photo.png\"",
                1 => "\"photo_1.png\"",
                _ => unreachable!(),
            }
        );
    }
    same("assets/photo.png");
    same("assets/photo_1.png");
    let source = a.join("X");
    fs::write(&source, b"stream").unwrap();
    for (index, expected) in ["X", "X_1"].into_iter().enumerate() {
        let actual = assets::import_asset(
            &store,
            "X",
            Content::Stream {
                source: fs::File::open(&source).unwrap(),
                max_bytes: u64::MAX,
            },
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            format!("{:?}", actual),
            match index {
                0 => "\"X\"",
                1 => "\"X_1\"",
                _ => unreachable!(),
            }
        );
        same(&format!("assets/{expected}"));
    }
    assert_eq!(
        format!(
            "{:?}",
            assets::import_asset_file(
                &store,
                "capture",
                Content::Stream {
                    source: fs::File::open(&source).unwrap(),
                    max_bytes: 100,
                },
            )
            .unwrap()
        ),
        "\"capture\""
    );
    same("assets/capture");
    let same_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for name in ["photo.png", "photo_1.png", "X", "X_1", "capture"] {
        fs::File::open(a.join("assets").join(name))
            .unwrap()
            .set_modified(same_time)
            .unwrap();
    }
    let new_orphans = assets::orphan_assets(&store)
        .into_iter()
        .map(|a| (a.name, a.size, a.modified))
        .collect::<Vec<_>>();
    assert!(!new_orphans
        .iter()
        .any(|(name, _, _)| name == "referenced.png"));
    assert_eq!(format!("{:?}", new_orphans), "[(\"X\", 6, Some(1700000000)), (\"X_1\", 6, Some(1700000000)), (\"capture\", 6, Some(1700000000)), (\"photo.png\", 3, Some(1700000000)), (\"photo_1.png\", 3, Some(1700000000))]");
    assert_eq!(
        format!(
            "{:?}",
            assets::trash_asset(&store, "missing")
                .unwrap_err()
                .to_string()
        ),
        "\"no such asset\""
    );
    assets::trash_asset(&store, "photo.png").unwrap();
    assert!(!a.join("assets/photo.png").exists());
    let trash_payloads = |root: &PathBuf| {
        let mut files = fs::read_dir(root.join("logseq/.tine-trash/assets"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                (
                    name.split_once("__").unwrap().1.to_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        files.sort();
        files
    };
    assert_eq!(
        format!("{:?}", trash_payloads(&a)),
        "[(\"photo.png\", [111, 110, 101])]"
    );

    for (index, bytes) in [b"first".as_slice(), b"second".as_slice()]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            format!(
                "{:?}",
                pdf::write_pdf_area_image(&store, "paper.pdf", 2, "area", 42, bytes).unwrap()
            ),
            match index {
                0 => "\"paper/2_area_42.png\"",
                1 => "\"paper/2_area_42.png\"",
                _ => unreachable!(),
            }
        );
        same("assets/paper/2_area_42.png");
    }
    assert_eq!(
        format!(
            "{:?}",
            pdf::write_pdf_area_image(&store, "paper.pdf", 2, "../escape", 42, b"x")
                .unwrap_err()
                .to_string()
        ),
        "\"bad asset name\""
    );
    assert_eq!(
        format!("{:?}", pdf::open_pdf(&store, "paper.pdf", "Paper").unwrap()),
        "PdfState { highlights: [], page: None, scale: None }"
    );
    same("assets/paper.edn");
    same("pages/hls__paper.md");
    pdf::write_pdf_view_state(&store, "paper.pdf", 3, 1.5).unwrap();
    same("assets/paper.edn");
    assert_eq!(
        format!(
            "{:?}",
            pdf::write_pdf_view_state(&store, "paper.pdf", 0, 1.0)
                .unwrap_err()
                .to_string()
        ),
        "\"invalid PDF view state\""
    );
    for (items, base) in [
        (vec![highlight("a")], vec![]),
        (vec![highlight("a"), highlight("b")], vec!["a".into()]),
    ] {
        pdf::write_highlights(&store, "paper.pdf", "Paper", &items, &base).unwrap();
        same("assets/paper.edn");
        same("pages/hls__paper.md");
    }
    assert_eq!(format!("{:?}", pdf::read_highlights(&store, "paper.pdf")), "[Highlight { id: \"a\", page: 1, position: Position { page: 1, bounding: Rect { top: 0.0, left: 0.0, width: 1.0, height: 1.0, source_width: None, source_height: None }, rects: [] }, color: \"yellow\", text: Some(\"a\"), image: None }, Highlight { id: \"b\", page: 1, position: Position { page: 1, bounding: Rect { top: 0.0, left: 0.0, width: 1.0, height: 1.0, source_width: None, source_height: None }, rects: [] }, color: \"yellow\", text: Some(\"b\"), image: None }]");
}

#[test]
fn trash_retries_external_write_without_losing_its_bytes() {
    let (root, store) = fixture("retry-trash");
    fs::write(root.join("assets/throwaway"), b"old").unwrap();
    store.inject_fault(FaultPoint::Stage2Mismatch);
    assets::trash_asset(&store, "throwaway").unwrap();
    assert!(!root.join("assets/throwaway").exists());
    let trash = root.join("logseq/.tine-trash/assets");
    assert!(fs::read_dir(trash)
        .unwrap()
        .flatten()
        .any(|entry| fs::read(entry.path()).unwrap() == b"external stage-2"));
}

#[test]
fn view_state_retries_external_sidecar_write_and_preserves_foreign_data() {
    let (root, store) = fixture("retry-view");
    pdf::write_pdf_view_state(&store, "paper.pdf", 1, 1.0).unwrap();
    store.inject_fault(FaultPoint::Stage2ValidSidecar);
    pdf::write_pdf_view_state(&store, "paper.pdf", 2, 1.5).unwrap();
    let edn = fs::read_to_string(root.join("assets/paper.edn")).unwrap();
    assert!(edn.contains("external") && edn.contains(":page 2"));
}

#[test]
fn area_image_retries_external_write_in_place() {
    let (root, store) = fixture("retry-image");
    pdf::write_pdf_area_image(&store, "paper.pdf", 2, "crop", 12, b"old").unwrap();
    store.inject_fault(FaultPoint::Stage2Mismatch);
    pdf::write_pdf_area_image(&store, "paper.pdf", 2, "crop", 12, b"new").unwrap();
    assert_eq!(
        fs::read(root.join("assets/paper/2_crop_12.png")).unwrap(),
        b"new"
    );
}

#[test]
fn highlights_retry_external_sidecar_write_and_preserve_foreign_data() {
    let (root, store) = fixture("retry-highlights");
    pdf::write_pdf_view_state(&store, "other.pdf", 1, 1.0).unwrap();
    store.inject_fault(FaultPoint::Stage2ValidSidecar);
    pdf::write_highlights(&store, "other.pdf", "Other", &[highlight("h")], &[]).unwrap();
    let edn = fs::read_to_string(root.join("assets/other.edn")).unwrap();
    assert!(edn.contains("external") && edn.contains("h"));
    assert!(root.join("pages/hls__other.md").exists());
}

#[test]
fn legacy_pdf_artifacts_stay_on_open_and_match_after_write_migration() {
    let (a, store) = fixture("migration-new");
    let pdf_name = "My Paper.pdf";
    let key = tine_core::pdf::asset_key(pdf_name);
    let legacy = tine_core::pdf::legacy_asset_key(pdf_name);
    let h = highlight("one");
    fs::write(
        a.join("assets").join(format!("{legacy}.edn")),
        tine_core::pdf::write_highlights(&[h.clone()], ""),
    )
    .unwrap();
    let page = tine_core::pdf::hls_page_document(pdf_name, "My Paper", &[h.clone()]);
    fs::write(
        a.join("pages").join(format!("hls__{legacy}.md")),
        tine_core::doc::serialize(&page),
    )
    .unwrap();
    store.scan_refresh().unwrap();
    pdf::open_pdf(&store, pdf_name, "My Paper").unwrap();
    assert!(!a.join("assets").join(format!("{key}.edn")).exists());
    assert_eq!(fs::read(a.join("assets").join(format!("{legacy}.edn"))).unwrap(), b"{:highlights [{:id \"one\" :page 1 :position {:page 1 :bounding {:top 0 :left 0 :width 1 :height 1} :rects ()} :content {:text \"one\"} :properties {:color \"yellow\"}}] :extra {}}\n");
    pdf::write_highlights(&store, pdf_name, "My Paper", &[h.clone()], &[h.id.clone()]).unwrap();
    for rel in [format!("assets/{key}.edn"), format!("pages/hls__{key}.md")] {
        assert_fixture_file(
            &a,
            "legacy_pdf_artifacts_stay_on_open_and_match_after_write_migration",
            "after_migration",
            &rel,
        );
    }
    assert!(!a.join("assets").join(format!("{legacy}.edn")).exists());
    assert!(!a.join("pages").join(format!("hls__{legacy}.md")).exists());
}

#[test]
fn blocked_trash_keeps_asset_and_legacy_error_text() {
    let (a, store) = fixture("blocked-new");
    fs::create_dir_all(a.join("logseq")).unwrap();
    fs::write(a.join("logseq/.tine-trash"), b"blocked").unwrap();
    fs::write(a.join("assets/photo.png"), b"safe").unwrap();
    store.scan_refresh().unwrap();
    let new_error = assets::trash_asset(&store, "photo.png")
        .unwrap_err()
        .to_string();
    assert_eq!(
        new_error,
        "could not create trash directory logseq/.tine-trash/assets: Not a directory (os error 20)"
    );
    assert_eq!(fs::read(a.join("assets/photo.png")).unwrap(), b"safe");
}

#[test]
fn malformed_sidecar_refusal_matches_legacy_text_and_keeps_bytes() {
    let (a, store) = fixture("malformed-new");
    let malformed = b"{:highlights []} trailing";
    fs::write(a.join("assets/paper.edn"), malformed).unwrap();
    store.scan_refresh().unwrap();
    assert_eq!(
        format!(
            "{:?}",
            pdf::open_pdf(&store, "paper.pdf", "Paper")
                .unwrap_err()
                .to_string()
        ),
        "\"highlight sidecar is malformed; refusing to replace it\""
    );
    assert_eq!(
        format!(
            "{:?}",
            pdf::write_highlights(&store, "paper.pdf", "Paper", &[highlight("h")], &[])
                .unwrap_err()
                .to_string()
        ),
        "\"highlight sidecar is malformed; refusing to replace it\""
    );
    assert_eq!(fs::read(a.join("assets/paper.edn")).unwrap(), malformed);
}

#[test]
fn annotation_notes_survive_update_with_legacy_bytes() {
    let h = highlight("one");
    let mut doc = tine_core::pdf::hls_page_document("paper.pdf", "Paper", &[h.clone()]);
    doc.roots[0]
        .children
        .push(tine_core::doc::DocBlock::new("private note"));
    let page = tine_core::doc::serialize(&doc);
    let (a, _) = fixture("notes-new");
    fs::write(
        a.join("assets/paper.edn"),
        tine_core::pdf::write_highlights(&[h.clone()], ""),
    )
    .unwrap();
    fs::write(a.join("pages/hls__paper.md"), &page).unwrap();
    let store = Store::open(&a, Default::default()).unwrap().0;
    pdf::write_highlights(&store, "paper.pdf", "Paper", &[h.clone()], &[h.id.clone()]).unwrap();
    let new_page = fs::read(a.join("pages/hls__paper.md")).unwrap();
    assert!(String::from_utf8_lossy(&new_page).contains("private note"));
    assert_eq!(new_page, b"file:: [Paper](../assets/paper.pdf)\nfile-path:: ../assets/paper.pdf\n\n- one\n  hl-page:: 1\n  hl-color:: yellow\n  ls-type:: annotation\n  id:: one\n\t- private note\n");
}

#[test]
fn empty_sanitized_pdf_key_keeps_legacy_crop_location() {
    let (a, store) = fixture("empty-key-new");
    let pdf_name = "??.pdf";
    assert_eq!(tine_core::pdf::asset_key(pdf_name), "");
    assert_eq!(
        pdf::write_pdf_area_image(&store, pdf_name, 1, "crop", 5, b"png").unwrap(),
        "/1_crop_5.png"
    );
    assert_eq!(fs::read(a.join("assets/1_crop_5.png")).unwrap(), b"png");
}

fn put(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn operation_result(result: &std::io::Result<()>, root: &std::path::Path) -> String {
    match result {
        Ok(()) => "ok".to_owned(),
        Err(error) => format!("{:?}: {}", error.kind(), error)
            .replace(&root.to_string_lossy().to_string(), "<root>"),
    }
}

#[test]
fn page_rename_matches_legacy_for_refs_namespace_alias_and_title() {
    for (index, (label, old_name, new_name, files, config)) in [
        (
            "plain",
            "x",
            "Next Name",
            vec![
                ("pages/x.md", "- own [[x]]\n"),
                ("pages/one.md", "- [[x]] #x #[[x]]\n"),
                ("pages/two.md", "tags:: x, [[x]], #x\n- [[x]]\n"),
                ("pages/three.org", "* [[x]] #x #[[x]]\n"),
                ("journals/2026_06_18.md", "- [[x]]\n"),
            ],
            None,
        ),
        (
            "namespace",
            "a",
            "b",
            vec![
                ("pages/a.md", "- [[a/child]]\n"),
                ("pages/a___child.md", "- [[a]]\n"),
                ("pages/ref.md", "- [[a/child]] [[a]]\n"),
            ],
            Some("{:file/name-format :triple-lowbar}"),
        ),
        (
            "alias-title",
            "Target",
            "Changed",
            vec![
                (
                    "pages/Target.md",
                    "title:: Display Target\nalias:: Alternate\n- [[Target]]\n",
                ),
                ("pages/ref.md", "- [[Target]] [[Alternate]]\n"),
            ],
            None,
        ),
        (
            "case-only",
            "target",
            "TARGET",
            vec![("pages/target.md", "- [[target]]\n")],
            None,
        ),
        (
            "reference-only",
            "Missing",
            "Found",
            vec![("pages/ref.md", "- [[Missing]] #Missing\n")],
            None,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (a, _) = fixture(&format!("rename-{label}-new"));
        for (rel, body) in &files {
            put(&a, rel, body);
        }
        if let Some(edn) = config {
            put(&a, "logseq/config.edn", edn);
        }
        let store = Store::open(&a, Default::default()).unwrap().0;
        let client = pages::rename_page_expected(&store, old_name, new_name, None);
        assert_eq!(
            format!("{:?}", operation_result(&client, &a)),
            match index {
                0 => "\"ok\"",
                1 => "\"ok\"",
                2 => "\"ok\"",
                3 => "\"ok\"",
                4 => "\"ok\"",
                _ => unreachable!(),
            }
        );
        assert_disk_tree(
            &a,
            "page_rename_matches_legacy_for_refs_namespace_alias_and_title",
            match index {
                0 => "simple",
                1 => "namespace",
                2 => "title_alias",
                3 => "case_change",
                4 => "tags",
                _ => unreachable!(),
            },
        );
        if label == "plain" {
            let moved = store.file_id(Area::Pages, "Next Name.md").unwrap();
            let disk_rev = store.read(&moved, None).unwrap().1;
            assert_eq!(
                store.page(&store.as_page(&moved).unwrap()).unwrap().rev,
                disk_rev
            );
        }
        if label == "case-only" {
            assert!(a.join("pages/target.md").exists());
            assert!(!a.join("pages/TARGET.md").exists());
        }
    }
}

#[test]
fn page_rename_refusals_match_legacy_and_keep_disk() {
    for (index, (label, files)) in [
        (
            "target-exists",
            vec![("pages/x.md", "- x\n"), ("pages/y.md", "- y\n")],
        ),
        (
            "org-h1",
            vec![
                ("pages/x.md", "- x\n"),
                ("pages/ref.org", "* Parent\n*** [[x]]\n"),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (a, _) = fixture(&format!("rename-refusal-{label}-new"));
        for (rel, body) in &files {
            put(&a, rel, body);
        }
        let store = Store::open(&a, Default::default()).unwrap().0;
        let before = disk_tree(&a);
        let to = if label == "target-exists" { "y" } else { "z" };
        let client = pages::rename_page_expected(&store, "x", to, None);
        assert_eq!(format!("{:?}", operation_result(&client, &a)), match index { 0 => "\"AlreadyExists: target page identity already exists elsewhere in the graph\"", 1 => "\"PermissionDenied: cannot rename: <root>/pages/ref.org is a read-only .org file (does not round-trip)\"", _ => unreachable!() });
        assert_eq!(disk_tree(&a), before, "{label} client changed disk");
        assert_disk_tree(
            &a,
            "page_rename_refusals_match_legacy_and_keep_disk",
            match index {
                0 => "existing_target",
                1 => "invalid_target",
                _ => unreachable!(),
            },
        );
    }
}

#[test]
fn page_merge_delete_and_rescue_match_legacy_bytes() {
    use tine_core::model::PageKind;
    let (a, _) = fixture("page-operations-new");
    put(
        &a,
        "pages/src.md",
        "alias:: Alias\ntags:: shared\n- moved\n",
    );
    put(&a, "pages/dst.md", "tags:: keep\n- kept\n");
    put(&a, "journals/Loose.md", "- rescued\n");
    put(&a, "pages/delete.md", "- gone\n");
    put(&a, "pages/ref.md", "- [[src]] and [[dst]]\n");
    let store = Store::open(&a, Default::default()).unwrap().0;
    pages::merge_pages(&store, "pages/src.md", "pages/dst.md").unwrap();
    assert_disk_tree(
        &a,
        "page_merge_delete_and_rescue_match_legacy_bytes",
        "merge",
    );
    pages::rename_file_to_page(&store, "journals/Loose.md", "Rescued").unwrap();
    assert_disk_tree(
        &a,
        "page_merge_delete_and_rescue_match_legacy_bytes",
        "rescue",
    );
    let id = store.file_id(Area::Pages, "delete.md").unwrap();
    let stale = store.read(&id, None).unwrap().1;
    put(&a, "pages/delete.md", "- later\n");
    let before_delete = disk_tree(&a);
    assert!(
        pages::delete_page_expected(&store, "delete", PageKind::Page, None, Some(&stale)).is_err()
    );
    assert_eq!(disk_tree(&a), before_delete, "stale delete changed disk");
    store.scan_refresh().unwrap();
    pages::delete_page_expected(&store, "delete", PageKind::Page, None, None).unwrap();
    assert_disk_tree(
        &a,
        "page_merge_delete_and_rescue_match_legacy_bytes",
        "delete_again",
    );
}

#[test]
fn merge_keeps_source_preamble_text_and_conflicting_properties() {
    let (root, _) = fixture("merge-source-preamble");
    put(
        &root,
        "pages/src.md",
        "alias:: Source alias\nnote:: only source\nfree text before bullets\n- moved\n",
    );
    put(&root, "pages/dst.md", "alias:: Destination alias\n- kept\n");
    let store = Store::open(&root, Default::default()).unwrap().0;

    pages::merge_pages(&store, "pages/src.md", "pages/dst.md").unwrap();
    let merged = std::fs::read_to_string(root.join("pages/dst.md")).unwrap();
    let parsed = tine_core::doc::parse(&merged);
    assert!(merged.contains("alias:: Destination alias"));
    assert!(merged.contains("note:: only source"));
    assert!(!merged.contains("Source page preamble"));
    assert!(
        parsed.roots.iter().any(|block| {
            block.raw() == "alias:: Source alias\nfree text before bullets"
        }),
        "I-4: merge must keep leftover source preamble lines verbatim in one block; exemplar pages::merge_pages"
    );
}

#[test]
fn org_merge_and_binary_rescue_match_legacy() {
    let (a, _) = fixture("org-merge-new");
    put(&a, "pages/src.org", "* moved\n");
    put(&a, "pages/dst.org", "* kept\n");
    put(&a, "pages/ref.md", "- [[src]] [[dst]]\n");
    fs::create_dir_all(a.join("journals")).unwrap();
    fs::write(a.join("journals/Loose.md"), [0xff, 0xfe, 0x00]).unwrap();
    let store = Store::open(&a, Default::default()).unwrap().0;
    let client = pages::merge_pages(&store, "pages/src.org", "pages/dst.org");
    assert_eq!(operation_result(&client, &a), "ok");
    assert_disk_tree(&a, "org_merge_and_binary_rescue_match_legacy", "org_merge");
    pages::rename_file_to_page(&store, "journals/Loose.md", "Rescued").unwrap();
    assert_disk_tree(
        &a,
        "org_merge_and_binary_rescue_match_legacy",
        "binary_rescue",
    );
}

#[test]
fn page_rename_retries_external_change_and_rolls_back_third_step_failure() {
    let (a, _) = fixture("rename-fault-retry");
    put(&a, "pages/x.md", "- [[x]]\n");
    put(&a, "pages/one.md", "- [[x]]\n");
    put(&a, "pages/two.md", "- [[x]]\n");
    let store = Store::open(&a, Default::default()).unwrap().0;
    store.inject_fault(FaultPoint::Stage2MismatchAt(1));
    pages::rename_page_expected(&store, "x", "y", None).unwrap();
    assert!(a.join("pages/y.md").exists());
    assert_eq!(
        fs::read(a.join("pages/two.md")).unwrap(),
        b"external stage-2"
    );
    assert_eq!(fs::read(a.join("pages/one.md")).unwrap(), b"- [[y]]\n");
    let (b, _) = fixture("rename-fault-rollback");
    put(&b, "pages/x.md", "- [[x]]\n");
    put(&b, "pages/one.md", "- [[x]]\n");
    put(&b, "pages/two.md", "- [[x]]\n");
    let store = Store::open(&b, Default::default()).unwrap().0;
    let before = disk_tree(&b);
    store.inject_fault(FaultPoint::MidStepIoAt(2));
    assert!(pages::rename_page_expected(&store, "x", "y", None).is_err());
    assert_eq!(disk_tree(&b), before, "failed commit changed disk");
}

/// OG `:block/refs` excludes `{{query}}` arguments, so a page that mentions the
/// renamed page only inside a query is not a referrer and keeps its bytes. v0.6.5
/// did this only with a warm reference index; its full-scan fallback rewrote them.
#[test]
fn page_rename_leaves_query_only_mentions_alone() {
    let (a, _) = fixture("rename-query-only");
    put(&a, "pages/x.md", "- body\n");
    put(
        &a,
        "pages/query.md",
        "- {{query (and (task TODO) [[x]])}}\n",
    );
    put(&a, "pages/ref.md", "- [[x]] and #x\n");
    let store = Store::open(&a, Default::default()).unwrap().0;
    pages::rename_page_expected(&store, "x", "y", None).unwrap();
    assert_eq!(
        fs::read(a.join("pages/query.md")).unwrap(),
        b"- {{query (and (task TODO) [[x]])}}\n"
    );
    assert_eq!(
        fs::read(a.join("pages/ref.md")).unwrap(),
        b"- [[y]] and #y\n"
    );
    assert!(a.join("pages/y.md").exists() && !a.join("pages/x.md").exists());
}

// `corpus_renames_match_legacy` retired after parity passed at 9c3d7c376.
// Receipt: /aux/koutecky/logseq/tine-agents/evidence/og/b15a/corpus-parity-receipt.txt
